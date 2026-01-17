use std::{
    borrow::Cow,
    cmp::max,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use ark_bn254::{Bn254, Fr, G1Projective as G1};
use ark_ec::pairing::Pairing;
use ark_grumpkin::Projective as G2;
use ark_poly_commit::kzg10::VerifierKey as KzgVerifierKey;
use ark_serialize::CanonicalDeserialize;
use arkworks_phase2::{
    accumulator::Accumulator, transcript::Transcript, utils::serialize_uncompressed,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use folding_schemes::{
    arith::Arith,
    commitment::{
        kzg::{ProverKey as KzgProverKey, KZG},
        pedersen::Pedersen,
        CommitmentScheme,
    },
    folding::nova::{decider_eth::DeciderEthCircuit, get_r1cs, PreprocessorParam},
    folding::traits::Dummy,
    frontend::FCircuit,
    transcript::poseidon::poseidon_canonical_config,
    FoldingScheme,
};
use bytes::Bytes;
use futures::{stream, StreamExt};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use rand::{
    rngs::{OsRng, StdRng},
    SeedableRng,
};
use reqwest::{header::CONTENT_LENGTH, Url};
use serde::{Deserialize, Serialize};
use url::Url as ParsedUrl;

use zkp::groth16::{params::Groth16Params, withdraw::SingleWithdrawCircuit};
use zkp::nova::{
    constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
    params::{DeciderParams, FParams, NovaParams, N},
    root_nova::RootCircuit,
    withdraw_nova::WithdrawCircuit,
};
use zkp::utils::poseidon::utils::{circom_poseidon2_config, circom_poseidon3_config};

const DEFAULT_PTAU_URL: &str =
    "https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_24.ptau";
const UPLOAD_CHUNK_SIZE: usize = 64 * 1024;

#[derive(Parser, Debug)]
#[command(author, version, about = "trusted setup CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    #[command(subcommand)]
    Ptau(PtauCommand),
    Contribute(ContributeArgs),
    Finalize(FinalizeArgs),
}

#[derive(Subcommand, Debug)]
enum PtauCommand {
    Download(PtauDownloadArgs),
}

#[derive(Args, Debug)]
struct PtauDownloadArgs {
    /// Ptau URL to download.
    #[arg(long, env = "TRUSTED_SETUP_PTAU_URL", default_value = DEFAULT_PTAU_URL)]
    url: String,

    /// Output path for the ptau file.
    #[arg(long, env = "TRUSTED_SETUP_PTAU_PATH")]
    output: Option<PathBuf>,

    /// Overwrite the existing file if present.
    #[arg(long, default_value_t = false)]
    force: bool,
}

#[derive(ValueEnum, Debug, Clone)]
enum CeremonyCircuit {
    #[value(name = "withdraw_local")]
    WithdrawLocal,
    #[value(name = "withdraw_global")]
    WithdrawGlobal,
    #[value(name = "decider_root", alias = "root")]
    DeciderRoot,
    #[value(name = "decider_withdraw_local")]
    DeciderWithdrawLocal,
    #[value(name = "decider_withdraw_global")]
    DeciderWithdrawGlobal,
}

impl CeremonyCircuit {
    fn as_str(&self) -> &'static str {
        match self {
            CeremonyCircuit::WithdrawLocal => "withdraw_local",
            CeremonyCircuit::WithdrawGlobal => "withdraw_global",
            CeremonyCircuit::DeciderRoot => "decider_root",
            CeremonyCircuit::DeciderWithdrawLocal => "decider_withdraw_local",
            CeremonyCircuit::DeciderWithdrawGlobal => "decider_withdraw_global",
        }
    }
}

#[derive(Args, Debug)]
struct ContributeArgs {
    /// Coordinator base URL.
    #[arg(long, env = "TRUSTED_SETUP_COORDINATOR_URL")]
    coordinator_url: String,

    /// Ceremony identifier.
    #[arg(long, env = "TRUSTED_SETUP_CEREMONY_ID")]
    ceremony_id: String,

    /// Circuit identifier for the ceremony.
    #[arg(long, env = "TRUSTED_SETUP_CIRCUIT", value_enum)]
    circuit: CeremonyCircuit,

    /// Ptau path.
    #[arg(long, env = "TRUSTED_SETUP_PTAU_PATH")]
    ptau_path: Option<PathBuf>,

    /// Optional seed for deterministic contribution.
    #[arg(long, env = "TRUSTED_SETUP_SEED")]
    seed: Option<String>,

    /// Deterministic seed for Pedersen params (decider circuits).
    #[arg(long, env = "TRUSTED_SETUP_PEDERSEN_SEED", default_value_t = 42)]
    pedersen_seed: u64,
}

#[derive(Args, Debug)]
struct FinalizeArgs {
    /// Circuit to finalize (must match the coordinator ceremony circuit).
    #[arg(long, env = "TRUSTED_SETUP_CIRCUIT", value_enum)]
    circuit: CeremonyCircuit,

    /// Ptau path.
    #[arg(long, env = "TRUSTED_SETUP_PTAU_PATH")]
    ptau_path: Option<PathBuf>,

    /// Output directory for artifacts.
    #[arg(long, env = "TRUSTED_SETUP_OUTPUT_DIR")]
    output_dir: Option<PathBuf>,

    /// Public base URL for latest.json and transcripts.
    #[arg(long, env = "TRUSTED_SETUP_PUBLIC_BASE_URL")]
    public_base_url: Option<String>,

    /// Ceremony id for the transcript.
    #[arg(long, env = "TRUSTED_SETUP_CEREMONY_ID")]
    ceremony_id: Option<String>,

    /// Explicit path or URL for the transcript.
    #[arg(long, env = "TRUSTED_SETUP_TRANSCRIPT")]
    transcript: Option<String>,

    /// Deterministic seed for Pedersen params.
    #[arg(long, env = "TRUSTED_SETUP_PEDERSEN_SEED", default_value_t = 42)]
    pedersen_seed: u64,
}

#[derive(Serialize)]
struct ParticipateRequest {
    circuit: String,
}

#[derive(Deserialize)]
struct ParticipateResponse {
    lease_id: String,
    participant_id: String,
    step: u64,
    input_url: String,
    output_url: String,
    contribution_url: String,
}

#[derive(Serialize)]
struct SubmitRequest {
    lease_id: String,
    participant_id: String,
}

#[derive(Debug)]
enum TranscriptSource {
    Url(ParsedUrl),
    Path(PathBuf),
}

#[derive(Deserialize)]
struct LatestMetadata {
    transcript_key: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Command::Ptau(PtauCommand::Download(args)) => download_ptau(args).await?,
        Command::Contribute(args) => contribute(args).await?,
        Command::Finalize(args) => finalize(args).await?,
    }

    Ok(())
}

async fn download_ptau(args: PtauDownloadArgs) -> Result<()> {
    let output = args.output.unwrap_or_else(default_ptau_path);
    if output.exists() && !args.force {
        bail!(
            "ptau already exists at {} (use --force to overwrite)",
            output.display()
        );
    }

    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let client = reqwest::Client::new();
    let resp = client
        .get(&args.url)
        .send()
        .await
        .with_context(|| format!("failed to download ptau from {}", args.url))?;
    if !resp.status().is_success() {
        bail!("ptau download failed with status {}", resp.status());
    }

    let tmp_path = output.with_extension("part");
    let progress = build_progress("downloading ptau", resp.content_length());
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .with_context(|| format!("failed to create {}", tmp_path.display()))?;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("failed to read ptau chunk")?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .context("failed to write ptau chunk")?;
        progress.inc(chunk.len() as u64);
    }
    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .context("failed to flush ptau output")?;
    drop(file);

    tokio::fs::rename(&tmp_path, &output)
        .await
        .with_context(|| format!("failed to move ptau to {}", output.display()))?;

    progress.finish_and_clear();
    println!("ptau downloaded to {}", output.display());
    Ok(())
}

async fn contribute(args: ContributeArgs) -> Result<()> {
    let ptau_path = args.ptau_path.unwrap_or_else(default_ptau_path);
    let accum = load_accumulator(&ptau_path)?;

    let base_url = Url::parse(&args.coordinator_url)
        .with_context(|| format!("invalid coordinator url {}", args.coordinator_url))?;

    let participate_url = base_url
        .join(&format!("/api/ceremonies/{}/participate", args.ceremony_id))
        .context("failed to build participate url")?;

    let client = reqwest::Client::new();
    let participate = client
        .post(participate_url)
        .json(&ParticipateRequest {
            circuit: args.circuit.as_str().to_string(),
        })
        .send()
        .await
        .context("participate request failed")?;

    if !participate.status().is_success() {
        bail!("participate failed with status {}", participate.status());
    }

    let participate: ParticipateResponse = participate
        .json()
        .await
        .context("failed to parse participate response")?;

    let input_bytes =
        download_bytes_with_progress(&client, &participate.input_url, "transcript").await?;

    let mut transcript = Transcript::<Bn254>::deserialize_uncompressed(&input_bytes[..])
        .context("failed to deserialize transcript")?;

    verify_transcript(&accum, &args.circuit, &transcript, args.pedersen_seed)
        .context("transcript verification failed")?;

    match &args.seed {
        Some(seed) => transcript
            .contribute_seed(seed.as_bytes())
            .context("failed to contribute using seed")?,
        None => {
            let mut rng = OsRng;
            transcript
                .contribute_rng(&mut rng)
                .context("failed to contribute using rng")?;
        }
    }

    transcript
        .verify()
        .context("transcript verification after contribution failed")?;

    let updated_bytes =
        serialize_uncompressed(&transcript).context("failed to serialize updated transcript")?;

    let contribution = transcript
        .contributions
        .last()
        .context("missing contribution data")?;
    let contribution_bytes =
        serialize_uncompressed(contribution).context("failed to serialize contribution")?;

    upload_bytes(&client, &participate.output_url, updated_bytes, "transcript")
        .await
        .context("failed to upload updated transcript")?;
    upload_bytes(
        &client,
        &participate.contribution_url,
        contribution_bytes,
        "contribution",
    )
        .await
        .context("failed to upload contribution")?;

    let submit_url = base_url
        .join(&format!("/api/ceremonies/{}/submit", args.ceremony_id))
        .context("failed to build submit url")?;

    let submit_resp = client
        .post(submit_url)
        .json(&SubmitRequest {
            lease_id: participate.lease_id,
            participant_id: participate.participant_id,
        })
        .send()
        .await
        .context("submit request failed")?;

    if !submit_resp.status().is_success() {
        bail!("submit failed with status {}", submit_resp.status());
    }

    println!("contribution submitted for step {}", participate.step);
    Ok(())
}

async fn finalize(args: FinalizeArgs) -> Result<()> {
    let ptau_path = args.ptau_path.unwrap_or_else(default_ptau_path);
    let output_dir = args
        .output_dir
        .unwrap_or_else(|| workspace_root().join("nova_artifacts"));
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;

    let accum = load_accumulator(&ptau_path)?;

    let transcript_source = resolve_transcript_source(
        args.transcript,
        args.ceremony_id.as_deref(),
        args.public_base_url.as_deref(),
    )
    .await?;
    let transcript = load_transcript(transcript_source).await?;

    match args.circuit {
        CeremonyCircuit::WithdrawLocal => {
            finalize_withdraw_groth16::<TRANSFER_TREE_HEIGHT>(
                "withdraw_local",
                &accum,
                &output_dir,
                transcript,
            )?;
        }
        CeremonyCircuit::WithdrawGlobal => {
            finalize_withdraw_groth16::<GLOBAL_TRANSFER_TREE_HEIGHT>(
                "withdraw_global",
                &accum,
                &output_dir,
                transcript,
            )?;
        }
        CeremonyCircuit::DeciderRoot => {
            finalize_decider::<RootCircuit<Fr>>(
                "root",
                &accum,
                &output_dir,
                transcript,
                args.pedersen_seed,
            )?;
        }
        CeremonyCircuit::DeciderWithdrawLocal => {
            finalize_decider::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>(
                "withdraw_local",
                &accum,
                &output_dir,
                transcript,
                args.pedersen_seed,
            )?;
        }
        CeremonyCircuit::DeciderWithdrawGlobal => {
            finalize_decider::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>(
                "withdraw_global",
                &accum,
                &output_dir,
                transcript,
                args.pedersen_seed,
            )?;
        }
    }

    Ok(())
}

/// Finalize SingleWithdrawCircuit Groth16 params from ceremony transcript.
fn finalize_withdraw_groth16<const DEPTH: usize>(
    prefix: &str,
    accum: &Accumulator<Bn254>,
    output_dir: &Path,
    transcript: Transcript<Bn254>,
) -> Result<()> {
    let withdraw_circuit = build_withdraw_circuit::<DEPTH>()?;
    transcript
        .verify_from_accumulator(accum, withdraw_circuit)
        .context("withdraw transcript verification failed")?;

    let groth16_params = groth16_from_transcript(transcript);
    emit_groth16_artifacts(prefix, output_dir, &groth16_params)?;

    println!("finalized {prefix} groth16 params");
    Ok(())
}

/// Finalize DeciderEthCircuit Groth16 params from ceremony transcript,
/// and generate Nova params from ptau (no ceremony needed for KZG).
fn finalize_decider<C>(
    prefix: &str,
    accum: &Accumulator<Bn254>,
    output_dir: &Path,
    transcript: Transcript<Bn254>,
    pedersen_seed: u64,
) -> Result<()>
where
    C: FCircuit<
        Fr,
        Params = (
            ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
            ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
        ),
    >,
{
    let poseidon2_config = circom_poseidon2_config::<Fr>();
    let poseidon3_config = circom_poseidon3_config();
    let f_params = (poseidon2_config, poseidon3_config);

    let nova_bundle = build_nova_bundle::<C>(accum, f_params, pedersen_seed)?;

    let decider_circuit = DeciderEthCircuit::<G1, G2>::dummy((
        nova_bundle.r1cs.clone(),
        nova_bundle.cf_r1cs.clone(),
        nova_bundle.cf_cs_pp.clone(),
        nova_bundle.poseidon_config.clone(),
        (),
        (),
        nova_bundle.state_len,
        2,
    ));
    transcript
        .verify_from_accumulator(accum, decider_circuit)
        .context("decider transcript verification failed")?;

    let groth16_params = groth16_from_transcript(transcript);

    emit_nova_artifacts(
        prefix,
        output_dir,
        &nova_bundle.params,
        &groth16_params,
        nova_bundle.state_len,
    )?;

    println!("finalized {prefix} nova and decider params");
    Ok(())
}

fn build_withdraw_circuit<const DEPTH: usize>() -> Result<SingleWithdrawCircuit<Fr, DEPTH>> {
    let poseidon2_config = circom_poseidon2_config::<Fr>();
    let poseidon3_config = circom_poseidon3_config();
    let zero = Fr::from(0u64);
    Ok(SingleWithdrawCircuit::<Fr, DEPTH> {
        poseidon2_params: poseidon2_config,
        poseidon3_params: poseidon3_config,
        merkle_root: Some(zero),
        recipient: Some(zero),
        withdraw_value: Some(zero),
        from: Some(zero),
        value: Some(zero),
        delta: Some(zero),
        secret: Some(zero),
        leaf_index: Some(0),
        siblings: [(); DEPTH].map(|_| Some(zero)),
    })
}

fn groth16_from_transcript(transcript: Transcript<Bn254>) -> Groth16Params {
    let pk = transcript.key.key;
    let vk = pk.vk.clone();
    Groth16Params { pk, vk }
}

struct NovaBundle<C: FCircuit<Fr>>
where
    FParams<C>: Clone,
{
    params: NovaParams<C>,
    r1cs: folding_schemes::arith::r1cs::R1CS<Fr>,
    cf_r1cs: folding_schemes::arith::r1cs::R1CS<<G2 as ark_ec::PrimeGroup>::ScalarField>,
    cf_cs_pp: folding_schemes::commitment::pedersen::Params<G2>,
    poseidon_config: ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
    state_len: usize,
}

fn build_nova_bundle<C>(
    accum: &Accumulator<Bn254>,
    f_params: FParams<C>,
    pedersen_seed: u64,
) -> Result<NovaBundle<C>>
where
    C: FCircuit<Fr>,
    FParams<C>: Clone,
{
    let poseidon_config = poseidon_canonical_config::<Fr>();
    let circuit = C::new(f_params.clone()).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let state_len = circuit.state_len();
    let (r1cs, cf_r1cs) = get_r1cs::<G1, G2, C>(&poseidon_config, circuit)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let kzg_len = max(r1cs.n_constraints(), r1cs.n_witnesses());
    let (cs_pp, cs_vp) = kzg_params_from_ptau(accum, kzg_len)?;

    let pedersen_len = max(cf_r1cs.n_constraints(), cf_r1cs.n_witnesses());
    let mut rng = StdRng::seed_from_u64(pedersen_seed);
    let (cf_cs_pp, cf_cs_vp) = Pedersen::<G2>::setup(&mut rng, pedersen_len)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let circuit = C::new(f_params.clone()).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let mut prep = PreprocessorParam::<G1, G2, C, KZG<'static, Bn254>, Pedersen<G2>, false>::new(
        poseidon_config.clone(),
        circuit,
    );
    prep.cs_pp = Some(cs_pp);
    prep.cs_vp = Some(cs_vp);
    prep.cf_cs_pp = Some(cf_cs_pp.clone());
    prep.cf_cs_vp = Some(cf_cs_vp);

    let mut rng = StdRng::seed_from_u64(0);
    let (pp, vp) =
        N::<C>::preprocess(&mut rng, &prep).map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let params = NovaParams { f_params, pp, vp };

    Ok(NovaBundle {
        params,
        r1cs,
        cf_r1cs,
        cf_cs_pp,
        poseidon_config,
        state_len,
    })
}

fn kzg_params_from_ptau(
    accum: &Accumulator<Bn254>,
    len: usize,
) -> Result<(KzgProverKey<'static, G1>, KzgVerifierKey<Bn254>)> {
    let needed = len + 1;
    if accum.tau_powers_g1.len() < needed {
        bail!(
            "ptau g1 powers too short: need {}, have {}",
            needed,
            accum.tau_powers_g1.len()
        );
    }
    if accum.tau_powers_g2.len() < 2 {
        bail!("ptau g2 powers too short: need at least 2");
    }
    if accum.alpha_tau_powers_g1.is_empty() {
        bail!("ptau alpha powers missing");
    }

    let powers_of_g = accum.tau_powers_g1[..needed].to_vec();
    let g = accum.tau_powers_g1[0];
    let gamma_g = accum.alpha_tau_powers_g1[0];
    let h = accum.tau_powers_g2[0];
    let beta_h = accum.tau_powers_g2[1];

    let prepared_h = <Bn254 as Pairing>::G2Prepared::from(h);
    let prepared_beta_h = <Bn254 as Pairing>::G2Prepared::from(beta_h);

    let vk = KzgVerifierKey {
        g,
        gamma_g,
        h,
        beta_h,
        prepared_h,
        prepared_beta_h,
    };

    let pp = KzgProverKey {
        powers_of_g: Cow::Owned(powers_of_g),
    };

    Ok((pp, vk))
}

fn emit_nova_artifacts<C>(
    prefix: &str,
    output_dir: &Path,
    nova_params: &NovaParams<C>,
    groth16_params: &Groth16Params,
    state_len: usize,
) -> Result<()>
where
    C: FCircuit<Fr>,
    FParams<C>: Clone,
{
    let (nova_pp_bytes, nova_vp_bytes) = nova_params
        .to_bytes()
        .context("failed to serialize nova params")?;

    let decider_pp = (groth16_params.pk.clone(), nova_params.pp.cs_pp.clone());
    let decider_vp = folding_schemes::folding::nova::decider_eth::VerifierParam {
        pp_hash: nova_params
            .vp
            .pp_hash()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?,
        snark_vp: groth16_params.vk.clone(),
        cs_vp: nova_params.vp.cs_vp.clone(),
    };
    let decider_params = DeciderParams::<C> {
        pp: decider_pp,
        vp: decider_vp,
    };

    let (decider_pp_bytes, decider_vp_bytes) = decider_params
        .to_bytes()
        .context("failed to serialize decider params")?;

    let pascal_case_prefix = to_pascal_case(prefix);
    let contract_name = format!("{}NovaDecider", pascal_case_prefix);
    let solidity = decider_params
        .verifier_solidity_code(state_len)
        .replace("NovaDecider", &contract_name);

    write_bytes(
        &output_dir.join(format!("{prefix}_nova_pp.bin")),
        &nova_pp_bytes,
    )?;
    write_bytes(
        &output_dir.join(format!("{prefix}_nova_vp.bin")),
        &nova_vp_bytes,
    )?;
    write_bytes(
        &output_dir.join(format!("{prefix}_decider_pp.bin")),
        &decider_pp_bytes,
    )?;
    write_bytes(
        &output_dir.join(format!("{prefix}_decider_vp.bin")),
        &decider_vp_bytes,
    )?;
    write_bytes(
        &output_dir.join(format!("{pascal_case_prefix}NovaDecider.sol")),
        solidity.as_bytes(),
    )?;

    Ok(())
}

fn emit_groth16_artifacts(
    prefix: &str,
    output_dir: &Path,
    groth16_params: &Groth16Params,
) -> Result<()> {
    let (pk_bytes, vk_bytes) = groth16_params
        .to_bytes()
        .context("failed to serialize groth16 params")?;

    let pascal_case_prefix = to_pascal_case(prefix);
    let contract_name = format!("{}Groth16Verifier", pascal_case_prefix);
    let solidity = groth16_params
        .verifier_solidity_code()
        .context("failed to render groth16 solidity")?
        .replace("Groth16Verifier", &contract_name);

    write_bytes(
        &output_dir.join(format!("{prefix}_groth16_pk.bin")),
        &pk_bytes,
    )?;
    write_bytes(
        &output_dir.join(format!("{prefix}_groth16_vk.bin")),
        &vk_bytes,
    )?;
    write_bytes(
        &output_dir.join(format!("{pascal_case_prefix}Groth16Verifier.sol")),
        solidity.as_bytes(),
    )?;

    Ok(())
}

fn verify_transcript(
    accum: &Accumulator<Bn254>,
    circuit: &CeremonyCircuit,
    transcript: &Transcript<Bn254>,
    pedersen_seed: u64,
) -> Result<()> {
    match circuit {
        CeremonyCircuit::WithdrawLocal => {
            let circuit = build_withdraw_circuit::<TRANSFER_TREE_HEIGHT>()?;
            transcript
                .verify_from_accumulator(accum, circuit)
                .context("withdraw_local verification failed")?;
        }
        CeremonyCircuit::WithdrawGlobal => {
            let circuit = build_withdraw_circuit::<GLOBAL_TRANSFER_TREE_HEIGHT>()?;
            transcript
                .verify_from_accumulator(accum, circuit)
                .context("withdraw_global verification failed")?;
        }
        CeremonyCircuit::DeciderRoot => {
            let circuit = build_decider_circuit::<RootCircuit<Fr>>(pedersen_seed)?;
            transcript
                .verify_from_accumulator(accum, circuit)
                .context("decider_root verification failed")?;
        }
        CeremonyCircuit::DeciderWithdrawLocal => {
            let circuit =
                build_decider_circuit::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>(pedersen_seed)?;
            transcript
                .verify_from_accumulator(accum, circuit)
                .context("decider_withdraw_local verification failed")?;
        }
        CeremonyCircuit::DeciderWithdrawGlobal => {
            let circuit = build_decider_circuit::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>(
                pedersen_seed,
            )?;
            transcript
                .verify_from_accumulator(accum, circuit)
                .context("decider_withdraw_global verification failed")?;
        }
    }
    Ok(())
}

fn build_decider_circuit<C>(pedersen_seed: u64) -> Result<DeciderEthCircuit<G1, G2>>
where
    C: FCircuit<
        Fr,
        Params = (
            ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
            ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
        ),
    >,
    FParams<C>: Clone,
{
    let poseidon_config = poseidon_canonical_config::<Fr>();
    let circuit = C::new(default_f_params::<C>()?).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let state_len = circuit.state_len();
    let (r1cs, cf_r1cs) = get_r1cs::<G1, G2, C>(&poseidon_config, circuit)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let pedersen_len = max(cf_r1cs.n_constraints(), cf_r1cs.n_witnesses());
    let mut rng = StdRng::seed_from_u64(pedersen_seed);
    let (cf_cs_pp, _) = Pedersen::<G2>::setup(&mut rng, pedersen_len)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    Ok(DeciderEthCircuit::<G1, G2>::dummy((
        r1cs,
        cf_r1cs,
        cf_cs_pp,
        poseidon_config,
        (),
        (),
        state_len,
        2,
    )))
}

fn default_f_params<C>() -> Result<(
    ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
    ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
)>
where
    C: FCircuit<
        Fr,
        Params = (
            ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
            ark_crypto_primitives::sponge::poseidon::PoseidonConfig<Fr>,
        ),
    >,
{
    let poseidon2_config = circom_poseidon2_config::<Fr>();
    let poseidon3_config = circom_poseidon3_config();
    Ok((poseidon2_config, poseidon3_config))
}

fn load_accumulator(path: &Path) -> Result<Accumulator<Bn254>> {
    Accumulator::<Bn254>::from_ptau_file(path)
        .with_context(|| format!("failed to load ptau from {}", path.display()))
}

async fn resolve_transcript_source(
    explicit: Option<String>,
    ceremony_id: Option<&str>,
    public_base_url: Option<&str>,
) -> Result<TranscriptSource> {
    if let Some(value) = explicit {
        return parse_transcript_source(&value);
    }

    let ceremony_id =
        ceremony_id.context("ceremony id is required when transcript path is not provided")?;
    let public_base_url = public_base_url
        .context("public base url is required when transcript path is not provided")?;

    let latest_url = build_latest_url(public_base_url, ceremony_id)?;
    let latest_resp = reqwest::get(latest_url)
        .await
        .context("failed to fetch latest metadata")?;
    if !latest_resp.status().is_success() {
        bail!(
            "latest metadata fetch failed with status {}",
            latest_resp.status()
        );
    }
    let latest_bytes = latest_resp
        .bytes()
        .await
        .context("failed to read latest metadata")?;

    let latest: LatestMetadata =
        serde_json::from_slice(&latest_bytes).context("failed to parse latest metadata")?;
    let transcript_url = build_object_url(public_base_url, &latest.transcript_key)?;
    Ok(TranscriptSource::Url(transcript_url))
}

fn parse_transcript_source(value: &str) -> Result<TranscriptSource> {
    if value.starts_with("http://") || value.starts_with("https://") {
        let url = ParsedUrl::parse(value).context("invalid transcript url")?;
        Ok(TranscriptSource::Url(url))
    } else {
        Ok(TranscriptSource::Path(PathBuf::from(value)))
    }
}

fn build_latest_url(base: &str, ceremony_id: &str) -> Result<ParsedUrl> {
    let base_url = normalize_base_url(base)?;
    let latest_path = format!("ceremonies/{}/latest.json", ceremony_id);
    base_url
        .join(&latest_path)
        .context("failed to join latest url")
}

fn build_object_url(base: &str, key: &str) -> Result<ParsedUrl> {
    let base_url = normalize_base_url(base)?;
    base_url.join(key).context("failed to join object url")
}

fn normalize_base_url(base: &str) -> Result<ParsedUrl> {
    let mut url = ParsedUrl::parse(base).context("invalid base url")?;
    if !url.path().ends_with('/') {
        let mut path = url.path().to_string();
        path.push('/');
        url.set_path(&path);
    }
    Ok(url)
}

async fn download_bytes_with_progress(
    client: &reqwest::Client,
    url: &str,
    label: &str,
) -> Result<Vec<u8>> {
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to download {}", label))?;
    if !resp.status().is_success() {
        bail!("{} download failed with status {}", label, resp.status());
    }

    let total = resp.content_length();
    let progress = build_progress(&format!("downloading {}", label), total);
    let mut stream = resp.bytes_stream();
    let mut bytes = match total.and_then(|len| usize::try_from(len).ok()) {
        Some(capacity) => Vec::with_capacity(capacity),
        None => Vec::new(),
    };
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("failed to read {} bytes", label))?;
        bytes.extend_from_slice(&chunk);
        progress.inc(chunk.len() as u64);
    }
    progress.finish_and_clear();
    Ok(bytes)
}

async fn load_transcript(source: TranscriptSource) -> Result<Transcript<Bn254>> {
    let bytes = match source {
        TranscriptSource::Url(url) => {
            let client = reqwest::Client::new();
            download_bytes_with_progress(&client, url.as_str(), "transcript").await?
        }
        TranscriptSource::Path(path) => tokio::fs::read(&path)
            .await
            .with_context(|| format!("failed to read {}", path.display()))?,
    };

    Transcript::<Bn254>::deserialize_uncompressed(&bytes[..])
        .context("failed to deserialize transcript")
}

async fn upload_bytes(
    client: &reqwest::Client,
    url: &str,
    body: Vec<u8>,
    label: &str,
) -> Result<()> {
    let total = body.len() as u64;
    let progress = build_progress(&format!("uploading {}", label), Some(total));
    let progress_handle = progress.clone();
    let stream = stream::unfold(
        (body, 0usize, progress_handle),
        |(body, offset, progress)| async move {
            if offset >= body.len() {
                None
            } else {
                let end = std::cmp::min(offset + UPLOAD_CHUNK_SIZE, body.len());
                let chunk = Bytes::copy_from_slice(&body[offset..end]);
                progress.inc((end - offset) as u64);
                Some((Ok::<Bytes, std::io::Error>(chunk), (body, end, progress)))
            }
        },
    );
    let resp = client
        .put(url)
        .header(CONTENT_LENGTH, total)
        .body(reqwest::Body::wrap_stream(stream))
        .send()
        .await;
    progress.finish_and_clear();
    let resp = resp?;
    if !resp.status().is_success() {
        bail!("upload failed with status {}", resp.status());
    }
    Ok(())
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => {
                    let mut result = String::new();
                    result.extend(first.to_uppercase());
                    result.push_str(chars.as_str());
                    result
                }
                None => String::new(),
            }
        })
        .collect()
}

fn default_ptau_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".cache")
            .join("zerc20")
            .join("ptau")
            .join("ppot_0080_24.ptau")
    } else {
        PathBuf::from("ptau").join("ppot_0080_24.ptau")
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn build_progress(message: &str, total_bytes: Option<u64>) -> ProgressBar {
    let progress = match total_bytes {
        Some(total) => ProgressBar::new(total),
        None => ProgressBar::new_spinner(),
    };
    progress.set_draw_target(ProgressDrawTarget::stderr());
    progress.enable_steady_tick(Duration::from_millis(120));
    if total_bytes.is_some() {
        let style = ProgressStyle::with_template(
            "{spinner:.green} {msg} [{bar:40}] {bytes}/{total_bytes} ({eta})",
        )
        .unwrap()
        .progress_chars("=>-")
        .tick_chars("|/-\\");
        progress.set_style(style);
    } else {
        let style =
            ProgressStyle::with_template("{spinner:.green} {msg} {bytes} downloaded")
                .unwrap()
                .tick_chars("|/-\\");
        progress.set_style(style);
    }
    progress.set_message(message.to_string());
    progress
}
