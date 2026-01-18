//! Common utilities and types for trusted setup ceremony.

use std::{cmp::max, path::Path};

use anyhow::{Context, Result};
use ark_bn254::{Bn254, Fr, G1Projective as G1};
use ark_grumpkin::Projective as G2;
use arkworks_phase2::{
    accumulator::{Accumulator, PreparedAccumulator},
    transcript::Transcript,
};
use folding_schemes::{
    arith::Arith,
    commitment::{pedersen::Pedersen, CommitmentScheme},
    folding::nova::{decider_eth::DeciderEthCircuit, get_r1cs},
    folding::traits::Dummy,
    frontend::FCircuit,
    transcript::poseidon::poseidon_canonical_config,
};
use rand::{rngs::StdRng, SeedableRng};
use serde::{Deserialize, Serialize};

use zkp::groth16::withdraw::SingleWithdrawCircuit;
use zkp::nova::{
    constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
    params::FParams,
    root_nova::RootCircuit,
    withdraw_nova::WithdrawCircuit,
};
use zkp::utils::poseidon::utils::{circom_poseidon2_config, circom_poseidon3_config};

/// Supported ceremony circuit types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CeremonyCircuit {
    WithdrawLocal,
    WithdrawGlobal,
    DeciderRoot,
    DeciderWithdrawLocal,
    DeciderWithdrawGlobal,
}

impl CeremonyCircuit {
    /// Parse circuit type from string.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "withdraw_local" => Ok(Self::WithdrawLocal),
            "withdraw_global" => Ok(Self::WithdrawGlobal),
            "decider_root" | "root" => Ok(Self::DeciderRoot),
            "decider_withdraw_local" => Ok(Self::DeciderWithdrawLocal),
            "decider_withdraw_global" => Ok(Self::DeciderWithdrawGlobal),
            _ => anyhow::bail!("unsupported circuit: {value}"),
        }
    }

    /// Get string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            CeremonyCircuit::WithdrawLocal => "withdraw_local",
            CeremonyCircuit::WithdrawGlobal => "withdraw_global",
            CeremonyCircuit::DeciderRoot => "decider_root",
            CeremonyCircuit::DeciderWithdrawLocal => "decider_withdraw_local",
            CeremonyCircuit::DeciderWithdrawGlobal => "decider_withdraw_global",
        }
    }

    /// Check if this is a Groth16 (withdraw) circuit.
    pub fn is_groth16(&self) -> bool {
        matches!(self, Self::WithdrawLocal | Self::WithdrawGlobal)
    }

    /// Check if this is a Nova decider circuit.
    pub fn is_decider(&self) -> bool {
        !self.is_groth16()
    }

    /// Get the required PTAU power for this circuit.
    pub fn ptau_power(&self) -> u8 {
        match self {
            // Groth16 circuits use smaller PTAU
            Self::WithdrawLocal | Self::WithdrawGlobal => 14,
            // Decider circuits need larger PTAU
            Self::DeciderRoot | Self::DeciderWithdrawLocal | Self::DeciderWithdrawGlobal => 24,
        }
    }

    /// Get all available circuit types.
    pub fn all() -> &'static [CeremonyCircuit] {
        &[
            CeremonyCircuit::WithdrawLocal,
            CeremonyCircuit::WithdrawGlobal,
            CeremonyCircuit::DeciderRoot,
            CeremonyCircuit::DeciderWithdrawLocal,
            CeremonyCircuit::DeciderWithdrawGlobal,
        ]
    }
}

impl std::fmt::Display for CeremonyCircuit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for CeremonyCircuit {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

/// Metadata for the latest transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestMetadata {
    pub step: u64,
    pub transcript_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contribution_key: Option<String>,
    pub updated_at: u64,
}

/// Load PTAU accumulator from file.
pub fn load_accumulator(path: &Path) -> Result<Accumulator<Bn254>> {
    Accumulator::<Bn254>::from_ptau_file(path)
        .with_context(|| format!("failed to load ptau from {}", path.display()))
}

/// Build initial transcript for a ceremony circuit.
pub fn build_initial_transcript(
    accum: &Accumulator<Bn254>,
    circuit: CeremonyCircuit,
    pedersen_seed: u64,
) -> Result<Transcript<Bn254>> {
    match circuit {
        CeremonyCircuit::WithdrawLocal => {
            let c = build_withdraw_circuit::<TRANSFER_TREE_HEIGHT>()?;
            Transcript::new_from_accumulator(accum, c).map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::WithdrawGlobal => {
            let c = build_withdraw_circuit::<GLOBAL_TRANSFER_TREE_HEIGHT>()?;
            Transcript::new_from_accumulator(accum, c).map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::DeciderRoot => {
            let c = build_decider_circuit::<RootCircuit<Fr>>(pedersen_seed)?;
            Transcript::new_from_accumulator(accum, c).map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::DeciderWithdrawLocal => {
            let c =
                build_decider_circuit::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>(pedersen_seed)?;
            Transcript::new_from_accumulator(accum, c).map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::DeciderWithdrawGlobal => {
            let c = build_decider_circuit::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>(
                pedersen_seed,
            )?;
            Transcript::new_from_accumulator(accum, c).map_err(|e| anyhow::anyhow!(e.to_string()))
        }
    }
}

/// Build initial transcript using a cached PreparedAccumulator.
/// This is much faster than build_initial_transcript when the cache exists.
///
/// If `cache_path` is None, uses a default path based on ptau_power and domain_size,
/// allowing cache sharing across circuits with the same domain size.
pub fn build_initial_transcript_cached(
    accum: &Accumulator<Bn254>,
    circuit: CeremonyCircuit,
    pedersen_seed: u64,
    cache_path: Option<&Path>,
) -> Result<Transcript<Bn254>> {
    // Get the required domain size for the circuit
    let domain_size = get_domain_size_for_circuit(circuit, pedersen_seed)?;
    let ptau_power = circuit.ptau_power();

    // Determine the cache path (use default if not provided)
    let default_cache_path = prepared_accum_cache_path(ptau_power, domain_size);
    let cache_path = cache_path.unwrap_or(&default_cache_path);

    eprintln!(
        "PreparedAccumulator cache path: {} (ptau_power={}, domain_size={})",
        cache_path.display(),
        ptau_power,
        domain_size
    );

    // Try to load cached PreparedAccumulator
    let prepared = if cache_path.exists() {
        eprintln!("Loading cached PreparedAccumulator...");
        match PreparedAccumulator::load(cache_path) {
            Ok(cached) => {
                // Verify the cached accumulator has the right size
                let (valid, len) = cached.check_pow_len();
                if valid && len == domain_size {
                    eprintln!("Cached PreparedAccumulator loaded (size={})", len);
                    cached
                } else {
                    eprintln!(
                        "Cached PreparedAccumulator size mismatch (expected {}, got {}), regenerating...",
                        domain_size, len
                    );
                    generate_and_save_prepared(accum, domain_size, cache_path)?
                }
            }
            Err(e) => {
                eprintln!(
                    "Failed to load cached PreparedAccumulator: {}, regenerating...",
                    e
                );
                generate_and_save_prepared(accum, domain_size, cache_path)?
            }
        }
    } else {
        eprintln!("PreparedAccumulator cache not found, generating...");
        generate_and_save_prepared(accum, domain_size, cache_path)?
    };

    // Build transcript from prepared accumulator
    build_transcript_from_prepared(&prepared, circuit, pedersen_seed)
}

/// Generate and save a PreparedAccumulator to the specified path.
fn generate_and_save_prepared(
    accum: &Accumulator<Bn254>,
    domain_size: usize,
    cache_path: &Path,
) -> Result<PreparedAccumulator<Bn254>> {
    let prepared = accum
        .prepare_with_size(domain_size)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // Create parent directories if needed
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    eprintln!("Saving PreparedAccumulator to cache...");
    prepared
        .save(cache_path)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    Ok(prepared)
}

/// Get the required domain size for a circuit.
fn get_domain_size_for_circuit(circuit: CeremonyCircuit, pedersen_seed: u64) -> Result<usize> {
    use ark_poly::{EvaluationDomain, Radix2EvaluationDomain};

    let total_constraints = match circuit {
        CeremonyCircuit::WithdrawLocal => {
            let c = build_withdraw_circuit::<TRANSFER_TREE_HEIGHT>()?;
            get_total_constraints(c)?
        }
        CeremonyCircuit::WithdrawGlobal => {
            let c = build_withdraw_circuit::<GLOBAL_TRANSFER_TREE_HEIGHT>()?;
            get_total_constraints(c)?
        }
        CeremonyCircuit::DeciderRoot => {
            let c = build_decider_circuit::<RootCircuit<Fr>>(pedersen_seed)?;
            get_total_constraints(c)?
        }
        CeremonyCircuit::DeciderWithdrawLocal => {
            let c =
                build_decider_circuit::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>(pedersen_seed)?;
            get_total_constraints(c)?
        }
        CeremonyCircuit::DeciderWithdrawGlobal => {
            let c = build_decider_circuit::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>(
                pedersen_seed,
            )?;
            get_total_constraints(c)?
        }
    };

    let domain = Radix2EvaluationDomain::<Fr>::new(total_constraints)
        .ok_or_else(|| anyhow::anyhow!("domain size too large"))?;

    Ok(domain.size())
}

fn get_total_constraints<C: ark_relations::gr1cs::ConstraintSynthesizer<Fr>>(
    circuit: C,
) -> Result<usize> {
    use ark_relations::gr1cs::{ConstraintSystem, R1CS_PREDICATE_LABEL};

    let cs = ConstraintSystem::new_ref();
    circuit.generate_constraints(cs.clone())?;
    cs.finalize();

    let num_constraints = cs
        .get_predicates_num_constraints(R1CS_PREDICATE_LABEL)
        .ok_or_else(|| anyhow::anyhow!("missing R1CS predicate"))?;
    let num_instance_variables = cs.num_instance_variables();

    Ok(num_constraints + num_instance_variables)
}

fn build_transcript_from_prepared(
    prepared: &PreparedAccumulator<Bn254>,
    circuit: CeremonyCircuit,
    pedersen_seed: u64,
) -> Result<Transcript<Bn254>> {
    match circuit {
        CeremonyCircuit::WithdrawLocal => {
            let c = build_withdraw_circuit::<TRANSFER_TREE_HEIGHT>()?;
            Transcript::new_from_prepared_accumulator(prepared, c)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::WithdrawGlobal => {
            let c = build_withdraw_circuit::<GLOBAL_TRANSFER_TREE_HEIGHT>()?;
            Transcript::new_from_prepared_accumulator(prepared, c)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::DeciderRoot => {
            let c = build_decider_circuit::<RootCircuit<Fr>>(pedersen_seed)?;
            Transcript::new_from_prepared_accumulator(prepared, c)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::DeciderWithdrawLocal => {
            let c =
                build_decider_circuit::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>(pedersen_seed)?;
            Transcript::new_from_prepared_accumulator(prepared, c)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::DeciderWithdrawGlobal => {
            let c = build_decider_circuit::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>(
                pedersen_seed,
            )?;
            Transcript::new_from_prepared_accumulator(prepared, c)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        }
    }
}

/// Verify transcript against accumulator for a given circuit.
pub fn verify_transcript(
    accum: &Accumulator<Bn254>,
    circuit: CeremonyCircuit,
    transcript: &Transcript<Bn254>,
    pedersen_seed: u64,
) -> Result<()> {
    match circuit {
        CeremonyCircuit::WithdrawLocal => {
            let c = build_withdraw_circuit::<TRANSFER_TREE_HEIGHT>()?;
            transcript
                .verify_from_accumulator(accum, c)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        CeremonyCircuit::WithdrawGlobal => {
            let c = build_withdraw_circuit::<GLOBAL_TRANSFER_TREE_HEIGHT>()?;
            transcript
                .verify_from_accumulator(accum, c)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        CeremonyCircuit::DeciderRoot => {
            let c = build_decider_circuit::<RootCircuit<Fr>>(pedersen_seed)?;
            transcript
                .verify_from_accumulator(accum, c)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        CeremonyCircuit::DeciderWithdrawLocal => {
            let c =
                build_decider_circuit::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>(pedersen_seed)?;
            transcript
                .verify_from_accumulator(accum, c)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        CeremonyCircuit::DeciderWithdrawGlobal => {
            let c = build_decider_circuit::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>(
                pedersen_seed,
            )?;
            transcript
                .verify_from_accumulator(accum, c)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
    }
    Ok(())
}

/// Verify transcript against a pre-computed initial transcript.
/// This is much faster than verify_transcript as it skips the expensive
/// IFFT and MSM computations needed to regenerate the initial transcript.
pub fn verify_transcript_from_initial(
    initial_transcript: &Transcript<Bn254>,
    transcript: &Transcript<Bn254>,
) -> Result<()> {
    transcript
        .verify_from_initial_transcript(initial_transcript)
        .map_err(|e| anyhow::anyhow!(e.to_string()))
}

/// Build a SingleWithdrawCircuit for Groth16.
pub fn build_withdraw_circuit<const DEPTH: usize>() -> Result<SingleWithdrawCircuit<Fr, DEPTH>> {
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

/// Build a DeciderEthCircuit for Nova.
pub fn build_decider_circuit<C>(pedersen_seed: u64) -> Result<DeciderEthCircuit<G1, G2>>
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

/// Get default FCircuit parameters.
pub fn default_f_params<C>() -> Result<(
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

/// Get default PTAU cache path for a given power.
pub fn ptau_path_for_power(power: u8) -> std::path::PathBuf {
    let filename = format!("ppot_0080_{}.ptau", power);
    if let Ok(home) = std::env::var("HOME") {
        std::path::PathBuf::from(home)
            .join(".cache")
            .join("zerc20")
            .join("ptau")
            .join(filename)
    } else {
        std::path::PathBuf::from("ptau").join(filename)
    }
}

/// Get default PTAU cache path (for backwards compatibility, uses power 24).
pub fn default_ptau_path() -> std::path::PathBuf {
    ptau_path_for_power(24)
}

/// Get PTAU cache path for a given circuit.
pub fn ptau_path_for_circuit(circuit: CeremonyCircuit) -> std::path::PathBuf {
    ptau_path_for_power(circuit.ptau_power())
}

/// Get default prepared accumulator cache path for a given PTAU power and domain size.
/// This allows sharing the cache across circuits with the same domain size.
pub fn prepared_accum_cache_path(ptau_power: u8, domain_size: usize) -> std::path::PathBuf {
    // domain_size is always a power of 2, so express it as 2^n for readability
    let domain_log2 = domain_size.ilog2();
    let filename = format!("ppot_{}_2pow{}.bin", ptau_power, domain_log2);
    if let Ok(home) = std::env::var("HOME") {
        std::path::PathBuf::from(home)
            .join(".cache")
            .join("zerc20")
            .join("prepared_accum")
            .join(filename)
    } else {
        std::path::PathBuf::from("prepared_accum").join(filename)
    }
}

/// Get default initial transcript path for a given circuit.
pub fn initial_transcript_path(circuit: CeremonyCircuit) -> std::path::PathBuf {
    let filename = format!("{}_initial_transcript.bin", circuit.as_str());
    if let Ok(home) = std::env::var("HOME") {
        std::path::PathBuf::from(home)
            .join(".cache")
            .join("zerc20")
            .join("transcripts")
            .join(filename)
    } else {
        std::path::PathBuf::from("transcripts").join(filename)
    }
}

/// Generate S3 key for transcript.
pub fn transcript_key(ceremony_id: &str, step: u64) -> String {
    format!("ceremonies/{}/transcripts/{}.bin", ceremony_id, step)
}

/// Generate S3 key for contribution.
pub fn contribution_key(ceremony_id: &str, step: u64) -> String {
    format!("ceremonies/{}/contributions/{}.bin", ceremony_id, step)
}

/// Generate S3 key for latest metadata.
pub fn latest_key(ceremony_id: &str) -> String {
    format!("ceremonies/{}/latest.json", ceremony_id)
}

/// Supported PTAU powers.
pub const SUPPORTED_PTAU_POWERS: &[u8] = &[14, 24];

/// PTAU URL for power 14 (used by withdraw_local/global groth16).
pub const PTAU_URL_14: &str =
    "https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_14.ptau";

/// PTAU URL for power 24 (used by decider circuits).
pub const PTAU_URL_24: &str =
    "https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_24.ptau";

/// Default PTAU download URL (for backwards compatibility, uses power 24).
pub const DEFAULT_PTAU_URL: &str = PTAU_URL_24;

/// Get PTAU download URL for a given power.
pub fn ptau_url_for_power(power: u8) -> Option<&'static str> {
    match power {
        14 => Some(PTAU_URL_14),
        24 => Some(PTAU_URL_24),
        _ => None,
    }
}

/// Get PTAU download URL for a given circuit.
pub fn ptau_url_for_circuit(circuit: CeremonyCircuit) -> &'static str {
    ptau_url_for_power(circuit.ptau_power()).expect("circuit uses supported ptau power")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_parse() {
        assert_eq!(
            CeremonyCircuit::parse("withdraw_local").unwrap(),
            CeremonyCircuit::WithdrawLocal
        );
        assert_eq!(
            CeremonyCircuit::parse("decider_root").unwrap(),
            CeremonyCircuit::DeciderRoot
        );
        assert_eq!(
            CeremonyCircuit::parse("root").unwrap(),
            CeremonyCircuit::DeciderRoot
        );
        assert!(CeremonyCircuit::parse("invalid").is_err());
    }

    #[test]
    fn test_circuit_as_str() {
        assert_eq!(CeremonyCircuit::WithdrawLocal.as_str(), "withdraw_local");
        assert_eq!(CeremonyCircuit::DeciderRoot.as_str(), "decider_root");
    }
}
