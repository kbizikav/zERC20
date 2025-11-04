use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use ark_bn254::Fr;
use ark_crypto_primitives::sponge::poseidon::PoseidonConfig;
use folding_schemes::frontend::FCircuit;
use rand::{SeedableRng, rngs::StdRng};

use zkp::{
    groth16::{params::Groth16Params, withdraw::SingleWithdrawCircuit},
    nova::{
        constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
        params::{DeciderParams, FParams, NovaParams},
        root_nova::RootCircuit,
        withdraw_nova::WithdrawCircuit,
    },
    utils::poseidon::utils::circom_poseidon_config,
};

fn main() -> Result<()> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("failed to locate workspace root directory")?
        .to_path_buf();

    let artifacts_dir = workspace_root.join("nova_artifacts");
    fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;

    let poseidon_config = circom_poseidon_config::<Fr>();

    generate_groth16_artifacts::<TRANSFER_TREE_HEIGHT>(
        "withdraw_local",
        &artifacts_dir,
        &poseidon_config,
    )?;
    println!("Generated local withdraw groth16 artifacts");

    generate_groth16_artifacts::<GLOBAL_TRANSFER_TREE_HEIGHT>(
        "withdraw_global",
        &artifacts_dir,
        &poseidon_config,
    )?;
    println!("Generated global withdraw nova artifacts");

    generate_nova_artifacts::<RootCircuit<Fr>>("root", &artifacts_dir, poseidon_config.clone())?;
    println!("Generated root nova artifacts");

    generate_nova_artifacts::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>(
        "withdraw_local",
        &artifacts_dir,
        poseidon_config.clone(),
    )?;
    println!("Generated local withdraw nova artifacts");

    generate_nova_artifacts::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>(
        "withdraw_global",
        &artifacts_dir,
        poseidon_config.clone(),
    )?;

    println!("Generated global withdraw groth16 artifacts");

    println!("All artifacts saved under {}", artifacts_dir.display());

    Ok(())
}

fn generate_nova_artifacts<C>(
    prefix: &str,
    output_dir: &std::path::Path,
    f_params: FParams<C>,
) -> Result<()>
where
    C: FCircuit<Fr>,
    FParams<C>: Clone,
{
    let mut rng = StdRng::seed_from_u64(42);

    let nova_params = NovaParams::<C>::rand(f_params.clone(), &mut rng)?;
    let state_len = nova_params.state_len()?;
    let decider_params = DeciderParams::<C>::rand(&mut rng, &nova_params)?;

    let (nova_pp_bytes, nova_vp_bytes) = nova_params.to_bytes()?;
    let (decider_pp_bytes, decider_vp_bytes) = decider_params.to_bytes()?;
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

fn generate_groth16_artifacts<const DEPTH: usize>(
    prefix: &str,
    output_dir: &std::path::Path,
    poseidon_config: &PoseidonConfig<Fr>,
) -> Result<()> {
    let mut rng = StdRng::seed_from_u64(42);
    let circuit = SingleWithdrawCircuit::<Fr, DEPTH>::new(poseidon_config.clone());
    let params = Groth16Params::rand(&mut rng, circuit.clone())
        .with_context(|| format!("failed groth16 setup for {prefix}"))?;

    let (pk_bytes, vk_bytes) = params
        .to_bytes()
        .with_context(|| format!("failed to serialize groth16 params for {prefix}"))?;

    let pascal_case_prefix = to_pascal_case(prefix);
    let contract_name = format!("{}Groth16Verifier", pascal_case_prefix);
    let solidity = params
        .verifier_solidity_code()
        .with_context(|| format!("failed to render solidity verifier for {prefix}"))?
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

fn write_bytes(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
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
