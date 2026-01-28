use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use ark_bn254::Fr;
use ark_crypto_primitives::sponge::poseidon::PoseidonConfig;
use folding_schemes::frontend::FCircuit;
use rand::{rngs::StdRng, SeedableRng};

use zerc20_zkp::{
    groth16::{params::Groth16Params, withdraw::SingleWithdrawCircuit},
    nova::{
        constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
        params::{DeciderParams, FParams, NovaParams},
        root_nova::RootCircuit,
        withdraw_nova::WithdrawCircuit,
    },
    utils::poseidon::utils::{circom_poseidon2_config, circom_poseidon3_config},
};

use crate::manifest::{create_artifact_entry, CircuitArtifacts, Manifest};

/// Generate all circuit artifacts.
pub fn generate(artifacts_dir: &Path, version: &str, seed: Option<u64>) -> Result<()> {
    // Warn about fixed seed
    if let Some(s) = seed {
        log::warn!("Using fixed seed {}. This is NOT recommended for production use!", s);
        log::warn!("Fixed seeds make the setup deterministic and may compromise security.");
    }

    fs::create_dir_all(artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;

    let poseidon2_config = circom_poseidon2_config::<Fr>();
    let poseidon3_config = circom_poseidon3_config();

    let mut manifest = Manifest::new(version.to_string());

    // Generate withdraw_local groth16 artifacts
    log::info!("Generating withdraw_local groth16 artifacts...");
    generate_groth16_artifacts::<TRANSFER_TREE_HEIGHT>(
        "withdraw_local",
        artifacts_dir,
        &poseidon2_config,
        &poseidon3_config,
        seed,
    )?;
    log::info!("Generated withdraw_local groth16 artifacts");

    // Generate withdraw_global groth16 artifacts
    log::info!("Generating withdraw_global groth16 artifacts...");
    generate_groth16_artifacts::<GLOBAL_TRANSFER_TREE_HEIGHT>(
        "withdraw_global",
        artifacts_dir,
        &poseidon2_config,
        &poseidon3_config,
        seed,
    )?;
    log::info!("Generated withdraw_global groth16 artifacts");

    // Generate root nova artifacts
    log::info!("Generating root nova artifacts...");
    generate_nova_artifacts::<RootCircuit<Fr>>(
        "root",
        artifacts_dir,
        (poseidon2_config.clone(), poseidon3_config.clone()),
        seed,
    )?;
    log::info!("Generated root nova artifacts");

    // Generate withdraw_local nova artifacts
    log::info!("Generating withdraw_local nova artifacts...");
    generate_nova_artifacts::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>(
        "withdraw_local",
        artifacts_dir,
        (poseidon2_config.clone(), poseidon3_config.clone()),
        seed,
    )?;
    log::info!("Generated withdraw_local nova artifacts");

    // Generate withdraw_global nova artifacts
    log::info!("Generating withdraw_global nova artifacts...");
    generate_nova_artifacts::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>(
        "withdraw_global",
        artifacts_dir,
        (poseidon2_config.clone(), poseidon3_config.clone()),
        seed,
    )?;
    log::info!("Generated withdraw_global nova artifacts");

    // Build manifest
    log::info!("Building manifest...");

    // Root circuit (no groth16)
    manifest.add_circuit(
        "root",
        CircuitArtifacts {
            nova_pp: create_artifact_entry(version, "root_nova_pp.bin", artifacts_dir)?,
            nova_vp: create_artifact_entry(version, "root_nova_vp.bin", artifacts_dir)?,
            decider_pp: create_artifact_entry(version, "root_decider_pp.bin", artifacts_dir)?,
            decider_vp: create_artifact_entry(version, "root_decider_vp.bin", artifacts_dir)?,
            groth16_pk: None,
            groth16_vk: None,
        },
    );

    // Withdraw local circuit
    manifest.add_circuit(
        "withdraw_local",
        CircuitArtifacts {
            nova_pp: create_artifact_entry(version, "withdraw_local_nova_pp.bin", artifacts_dir)?,
            nova_vp: create_artifact_entry(version, "withdraw_local_nova_vp.bin", artifacts_dir)?,
            decider_pp: create_artifact_entry(version, "withdraw_local_decider_pp.bin", artifacts_dir)?,
            decider_vp: create_artifact_entry(version, "withdraw_local_decider_vp.bin", artifacts_dir)?,
            groth16_pk: Some(create_artifact_entry(version, "withdraw_local_groth16_pk.bin", artifacts_dir)?),
            groth16_vk: Some(create_artifact_entry(version, "withdraw_local_groth16_vk.bin", artifacts_dir)?),
        },
    );

    // Withdraw global circuit
    manifest.add_circuit(
        "withdraw_global",
        CircuitArtifacts {
            nova_pp: create_artifact_entry(version, "withdraw_global_nova_pp.bin", artifacts_dir)?,
            nova_vp: create_artifact_entry(version, "withdraw_global_nova_vp.bin", artifacts_dir)?,
            decider_pp: create_artifact_entry(version, "withdraw_global_decider_pp.bin", artifacts_dir)?,
            decider_vp: create_artifact_entry(version, "withdraw_global_decider_vp.bin", artifacts_dir)?,
            groth16_pk: Some(create_artifact_entry(version, "withdraw_global_groth16_pk.bin", artifacts_dir)?),
            groth16_vk: Some(create_artifact_entry(version, "withdraw_global_groth16_vk.bin", artifacts_dir)?),
        },
    );

    // Save manifest
    let manifest_path = artifacts_dir.join("manifest.json");
    manifest.save(&manifest_path)?;
    log::info!("Manifest saved to {}", manifest_path.display());

    let digest = manifest.digest()?;
    log::info!("Manifest digest: {}", digest);

    log::info!("All artifacts generated in {}", artifacts_dir.display());

    Ok(())
}

fn generate_nova_artifacts<C>(
    prefix: &str,
    output_dir: &Path,
    f_params: FParams<C>,
    seed: Option<u64>,
) -> Result<()>
where
    C: FCircuit<Fr>,
    FParams<C>: Clone,
{
    let mut rng = create_rng(seed)?;

    let nova_params = NovaParams::<C>::rand(f_params.clone(), &mut rng)?;
    let decider_params = DeciderParams::<C>::rand(&mut rng, &nova_params)?;

    let (nova_pp_bytes, nova_vp_bytes) = nova_params.to_bytes()?;
    let (decider_pp_bytes, decider_vp_bytes) = decider_params.to_bytes()?;

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

    Ok(())
}

fn generate_groth16_artifacts<const DEPTH: usize>(
    prefix: &str,
    output_dir: &Path,
    poseidon2_config: &PoseidonConfig<Fr>,
    poseidon3_config: &PoseidonConfig<Fr>,
    seed: Option<u64>,
) -> Result<()> {
    let mut rng = create_rng(seed)?;
    let circuit =
        SingleWithdrawCircuit::<Fr, DEPTH>::new(poseidon2_config.clone(), poseidon3_config.clone());
    let params = Groth16Params::rand(&mut rng, circuit)
        .with_context(|| format!("failed groth16 setup for {prefix}"))?;

    let (pk_bytes, vk_bytes) = params
        .to_bytes()
        .with_context(|| format!("failed to serialize groth16 params for {prefix}"))?;

    write_bytes(
        &output_dir.join(format!("{prefix}_groth16_pk.bin")),
        &pk_bytes,
    )?;
    write_bytes(
        &output_dir.join(format!("{prefix}_groth16_vk.bin")),
        &vk_bytes,
    )?;

    Ok(())
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn create_rng(seed: Option<u64>) -> Result<StdRng> {
    match seed {
        Some(s) => Ok(StdRng::seed_from_u64(s)),
        None => {
            log::info!("Collecting entropy for secure random generation...");
            Ok(StdRng::from_entropy())
        }
    }
}
