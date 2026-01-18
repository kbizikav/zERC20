//! Common utilities and types for trusted setup ceremony.

use std::{cmp::max, path::Path};

use anyhow::{Context, Result};
use ark_bn254::{Bn254, Fr, G1Projective as G1};
use ark_grumpkin::Projective as G2;
use arkworks_phase2::{accumulator::Accumulator, transcript::Transcript};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
            Transcript::new_from_accumulator(accum, c)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::WithdrawGlobal => {
            let c = build_withdraw_circuit::<GLOBAL_TRANSFER_TREE_HEIGHT>()?;
            Transcript::new_from_accumulator(accum, c)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::DeciderRoot => {
            let c = build_decider_circuit::<RootCircuit<Fr>>(pedersen_seed)?;
            Transcript::new_from_accumulator(accum, c)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::DeciderWithdrawLocal => {
            let c =
                build_decider_circuit::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>(pedersen_seed)?;
            Transcript::new_from_accumulator(accum, c)
                .map_err(|e| anyhow::anyhow!(e.to_string()))
        }
        CeremonyCircuit::DeciderWithdrawGlobal => {
            let c = build_decider_circuit::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>(
                pedersen_seed,
            )?;
            Transcript::new_from_accumulator(accum, c)
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
