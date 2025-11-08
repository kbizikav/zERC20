use std::{env, path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use api_types::prover::{CircuitKind, JobRequest, JobStatus, JobStatusResponse, SubmitJobResponse};
use ark_bn254::Fr;
use ark_ff::Zero;
use ark_serialize::CanonicalSerialize;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::Utc;
use folding_schemes::FoldingScheme;
use rand::{SeedableRng, rngs::StdRng};
use reqwest::StatusCode;
use tokio::time::sleep;
use zkp::{
    nova::{
        constants::TRANSFER_TREE_HEIGHT,
        params::NovaParams,
        root_nova::{RootCircuit, RootExternalInputs},
    },
    utils::poseidon::utils::circom_poseidon_config,
};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const PROVER_URL_ENV: &str = "DECIDER_PROVER_URL";

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let base_url =
        env::var(PROVER_URL_ENV).with_context(|| format!("{PROVER_URL_ENV} is not set"))?;
    let base_url = base_url.trim_end_matches('/').to_owned();

    let client = reqwest::Client::new();
    let job_id = format!("dummy-root-{}", Utc::now().timestamp_millis());
    let ivc_proof = build_dummy_root_ivc_proof_base64()?;
    let request = JobRequest {
        job_id: job_id.clone(),
        circuit: CircuitKind::Root,
        ivc_proof,
    };

    let submit = submit_job(&client, &base_url, &request).await?;
    println!(
        "submitted job {} (status: {})",
        submit.job_id, submit.status
    );

    let final_status = wait_for_completion(&client, &base_url, &job_id).await?;
    match final_status.status {
        JobStatus::Completed => {
            println!("job {} completed", final_status.job_id);
            if let Some(result) = final_status.result {
                println!("proof (base64): {result}");
            }
        }
        JobStatus::Failed => {
            println!("job {} failed", final_status.job_id);
            if let Some(error) = final_status.error {
                println!("error: {error}");
            }
        }
        other => {
            println!(
                "job {} finished with unexpected status {other}",
                final_status.job_id
            );
        }
    }

    Ok(())
}

fn build_dummy_root_ivc_proof_base64() -> Result<String> {
    let nova_params = load_root_nova_params_from_artifacts()
        .context("failed to load nova params from artifacts")?;
    let state_len = nova_params
        .state_len()
        .context("root nova params must expose state length")?;
    let mut nova = nova_params
        .initial_nova(vec![Fr::zero(); state_len])
        .context("failed to initialize root nova instance")?;
    let mut step_rng = StdRng::seed_from_u64(0xBAD5EED);
    let external_input = RootExternalInputs::<Fr> {
        is_dummy: true,
        address: Fr::zero(),
        value: Fr::zero(),
        siblings: [Fr::zero(); TRANSFER_TREE_HEIGHT],
    };
    nova.prove_step(&mut step_rng, external_input.clone(), None)
        .context("failed to execute dummy root nova step")?;
    nova.prove_step(&mut step_rng, external_input, None)
        .context("failed to execute dummy root nova step")?;
    let ivc_proof = nova.ivc_proof();
    nova_params
        .verify(ivc_proof.clone())
        .context("failed to verify dummy root ivc proof")?;

    let mut bytes = Vec::new();
    ivc_proof
        .serialize_uncompressed(&mut bytes)
        .context("failed to serialize dummy ivc proof")?;
    Ok(BASE64_STANDARD.encode(bytes))
}

fn load_root_nova_params_from_artifacts() -> Result<NovaParams<RootCircuit<Fr>>> {
    const NOVA_ARTIFACTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../nova_artifacts");
    let artifacts_path = Path::new(NOVA_ARTIFACTS_DIR);
    load_root_nova_params(artifacts_path)
}

fn load_root_nova_params(dir: &Path) -> Result<NovaParams<RootCircuit<Fr>>> {
    let pp_path = dir.join("root_nova_pp.bin");
    let vp_path = dir.join("root_nova_vp.bin");
    let pp_bytes =
        std::fs::read(&pp_path).with_context(|| format!("failed to read {}", pp_path.display()))?;
    let vp_bytes =
        std::fs::read(&vp_path).with_context(|| format!("failed to read {}", vp_path.display()))?;
    let poseidon_params = circom_poseidon_config::<Fr>();
    NovaParams::<RootCircuit<Fr>>::from_bytes(poseidon_params, pp_bytes, vp_bytes)
        .context("failed to deserialize nova params from artifacts")
}

async fn submit_job(
    client: &reqwest::Client,
    base_url: &str,
    request: &JobRequest,
) -> Result<SubmitJobResponse> {
    let url = format!("{base_url}/jobs");
    let response = client.post(url).json(request).send().await?;
    response
        .error_for_status()
        .context("failed to submit job")?
        .json::<SubmitJobResponse>()
        .await
        .context("invalid submit response")
}

async fn wait_for_completion(
    client: &reqwest::Client,
    base_url: &str,
    job_id: &str,
) -> Result<JobStatusResponse> {
    let url = format!("{base_url}/jobs/{job_id}");
    loop {
        let response = client.get(&url).send().await?;
        if response.status() == StatusCode::NOT_FOUND {
            println!("job {job_id} not visible yet, retrying...");
            sleep(POLL_INTERVAL).await;
            continue;
        }

        if response.status() != StatusCode::OK {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            bail!("status query failed with {status}: {text}");
        }

        let status = response.json::<JobStatusResponse>().await?;
        println!("job {job_id} status: {}", status.status);
        match status.status {
            JobStatus::Queued | JobStatus::Processing => {}
            _ => return Ok(status),
        }
        sleep(POLL_INTERVAL).await;
    }
}
