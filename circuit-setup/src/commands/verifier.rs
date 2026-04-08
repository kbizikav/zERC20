// SPDX-License-Identifier: BUSL-1.1

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use ark_bn254::Fr;
use folding_schemes::frontend::FCircuit;

use zerc20_zkp::{
    groth16::params::Groth16Params,
    nova::{
        constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
        params::{DeciderParams, FParams},
        root_nova::RootCircuit,
        withdraw_nova::WithdrawCircuit,
    },
    utils::poseidon::utils::{circom_poseidon2_config, circom_poseidon3_config},
};

/// Generate Solidity verifiers from artifacts.
pub fn generate_verifiers(artifacts_dir: &Path, output_dir: Option<&Path>) -> Result<()> {
    let output_dir = output_dir.unwrap_or(artifacts_dir);

    // Create output directory if it doesn't exist
    if !output_dir.exists() {
        fs::create_dir_all(output_dir).with_context(|| {
            format!("failed to create output directory {}", output_dir.display())
        })?;
        log::info!("Created output directory: {}", output_dir.display());
    }

    let poseidon2_config = circom_poseidon2_config::<Fr>();
    let poseidon3_config = circom_poseidon3_config();

    // Generate Nova decider verifiers
    log::info!("Generating RootNovaDecider.sol...");
    generate_nova_verifier::<RootCircuit<Fr>>(
        "root",
        "RootNovaDecider",
        artifacts_dir,
        output_dir,
        (poseidon2_config.clone(), poseidon3_config.clone()),
    )?;

    log::info!("Generating WithdrawLocalNovaDecider.sol...");
    generate_nova_verifier::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>(
        "withdraw_local",
        "WithdrawLocalNovaDecider",
        artifacts_dir,
        output_dir,
        (poseidon2_config.clone(), poseidon3_config.clone()),
    )?;

    log::info!("Generating WithdrawGlobalNovaDecider.sol...");
    generate_nova_verifier::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>(
        "withdraw_global",
        "WithdrawGlobalNovaDecider",
        artifacts_dir,
        output_dir,
        (poseidon2_config.clone(), poseidon3_config.clone()),
    )?;

    // Generate Groth16 verifiers
    log::info!("Generating WithdrawLocalGroth16Verifier.sol...");
    generate_groth16_verifier(
        "withdraw_local",
        "WithdrawLocalGroth16Verifier",
        artifacts_dir,
        output_dir,
    )?;

    log::info!("Generating WithdrawGlobalGroth16Verifier.sol...");
    generate_groth16_verifier(
        "withdraw_global",
        "WithdrawGlobalGroth16Verifier",
        artifacts_dir,
        output_dir,
    )?;

    log::info!(
        "All Solidity verifiers generated in {}",
        output_dir.display()
    );

    Ok(())
}

fn generate_nova_verifier<C>(
    prefix: &str,
    contract_name: &str,
    artifacts_dir: &Path,
    output_dir: &Path,
    f_params: FParams<C>,
) -> Result<()>
where
    C: FCircuit<Fr>,
    FParams<C>: Clone,
{
    let circuit = C::new(f_params.clone())
        .map_err(|e| anyhow::anyhow!("failed to create circuit for {}: {}", prefix, e))?;
    let state_len = circuit.state_len();

    // Load decider params
    let decider_pp_path = artifacts_dir.join(format!("{}_decider_pp.bin", prefix));
    let decider_vp_path = artifacts_dir.join(format!("{}_decider_vp.bin", prefix));

    let decider_pp = fs::read(&decider_pp_path)
        .with_context(|| format!("failed to read {}", decider_pp_path.display()))?;
    let decider_vp = fs::read(&decider_vp_path)
        .with_context(|| format!("failed to read {}", decider_vp_path.display()))?;

    let decider_params = DeciderParams::<C>::from_bytes(decider_pp, decider_vp)
        .with_context(|| format!("failed to deserialize decider params for {}", prefix))?;

    // Generate Solidity code
    let solidity = decider_params
        .verifier_solidity_code(state_len)
        .replace("NovaDecider", contract_name);

    // Write to file
    let output_path = output_dir.join(format!("{}.sol", contract_name));
    fs::write(&output_path, solidity)
        .with_context(|| format!("failed to write {}", output_path.display()))?;

    log::info!("  Generated {}", output_path.display());

    Ok(())
}

fn generate_groth16_verifier(
    prefix: &str,
    contract_name: &str,
    artifacts_dir: &Path,
    output_dir: &Path,
) -> Result<()> {
    // Load groth16 params
    let pk_path = artifacts_dir.join(format!("{}_groth16_pk.bin", prefix));
    let vk_path = artifacts_dir.join(format!("{}_groth16_vk.bin", prefix));

    let pk = fs::read(&pk_path).with_context(|| format!("failed to read {}", pk_path.display()))?;
    let vk = fs::read(&vk_path).with_context(|| format!("failed to read {}", vk_path.display()))?;

    let params = Groth16Params::from_bytes(pk, vk)
        .with_context(|| format!("failed to deserialize groth16 params for {}", prefix))?;

    // Generate Solidity code
    let solidity = params
        .verifier_solidity_code()
        .with_context(|| format!("failed to generate solidity for {}", prefix))?
        .replace("Groth16Verifier", contract_name);

    // Write to file
    let output_path = output_dir.join(format!("{}.sol", contract_name));
    fs::write(&output_path, solidity)
        .with_context(|| format!("failed to write {}", output_path.display()))?;

    log::info!("  Generated {}", output_path.display());

    Ok(())
}
