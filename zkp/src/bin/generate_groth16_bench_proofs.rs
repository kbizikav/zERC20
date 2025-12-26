use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use ark_bn254::{Bn254, Fr};
use ark_crypto_primitives::sponge::poseidon::PoseidonConfig;
use ark_groth16::Groth16;
use ark_serialize::CanonicalSerialize;
use ark_snark::SNARK;
use ark_std::rand::{rngs::StdRng, SeedableRng};
use solidity_verifiers::utils::eth::ToEth;

use zkp::{
    circuits::burn_address::{compute_burn_address_from_secret, find_pow_nonce, secret_from_nonce},
    groth16::{params::Groth16Params, withdraw::SingleWithdrawCircuit},
    nova::constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
    utils::{
        poseidon::utils::{circom_poseidon2_config, circom_poseidon3_config},
        tree::gadgets::leaf_hash::compute_leaf_hash,
    },
};

fn main() -> Result<()> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("failed to locate workspace root directory")?
        .to_path_buf();

    let artifacts_dir = workspace_root.join("nova_artifacts");
    fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;

    let poseidon2_config = circom_poseidon2_config::<Fr>();
    let poseidon3_config = circom_poseidon3_config();

    generate_bench_proof::<TRANSFER_TREE_HEIGHT>(
        "withdraw_local",
        &artifacts_dir,
        &poseidon2_config,
        &poseidon3_config,
    )?;
    println!("Generated local withdraw groth16 bench proof");

    generate_bench_proof::<GLOBAL_TRANSFER_TREE_HEIGHT>(
        "withdraw_global",
        &artifacts_dir,
        &poseidon2_config,
        &poseidon3_config,
    )?;
    println!("Generated global withdraw groth16 bench proof");

    println!(
        "Groth16 bench proofs saved under {}",
        artifacts_dir.display()
    );

    Ok(())
}

fn generate_bench_proof<const DEPTH: usize>(
    prefix: &str,
    output_dir: &std::path::Path,
    poseidon2_config: &PoseidonConfig<Fr>,
    poseidon3_config: &PoseidonConfig<Fr>,
) -> Result<()> {
    // Load Groth16 parameters produced by `generate_circuit_artifacts`
    let pk_path = output_dir.join(format!("{prefix}_groth16_pk.bin"));
    let vk_path = output_dir.join(format!("{prefix}_groth16_vk.bin"));

    let pk_bytes =
        fs::read(&pk_path).with_context(|| format!("failed to read {}", pk_path.display()))?;
    let vk_bytes =
        fs::read(&vk_path).with_context(|| format!("failed to read {}", vk_path.display()))?;

    let params =
        Groth16Params::from_bytes(pk_bytes, vk_bytes).context("failed to deserialize Groth16 params")?;

    // Construct a concrete withdraw circuit instance (same pattern as the unit test)
    let recipient_value = Fr::from(321u64);
    let secret_seed = Fr::from(654u64);
    let nonce = find_pow_nonce(recipient_value, secret_seed);
    let secret_value = secret_from_nonce(secret_seed, nonce);
    let address_value = compute_burn_address_from_secret(recipient_value, secret_value)
        .expect("nonce should satisfy PoW");

    let from_value = Fr::from(42u64);
    let value_value = Fr::from(100u64);
    let delta_value = Fr::from(25u64);
    let withdraw_value_value = value_value - delta_value;
    let leaf_index_value: u64 = 3;

    // Simple deterministic sibling values based on depth
    let siblings_values: Vec<Fr> = (0..DEPTH)
        .map(|i| Fr::from(5u64 + i as u64))
        .collect();
    let siblings_array: [Fr; DEPTH] = siblings_values
        .try_into()
        .expect("incorrect siblings length");

    let leaf_value = compute_leaf_hash(from_value, address_value, value_value);
    let merkle_root_value =
        merkle_root_from_path(poseidon2_config, leaf_index_value, leaf_value, &siblings_array);

    let circuit = SingleWithdrawCircuit::<Fr, DEPTH> {
        poseidon2_params: poseidon2_config.clone(),
        poseidon3_params: poseidon3_config.clone(),
        merkle_root: Some(merkle_root_value),
        recipient: Some(recipient_value),
        withdraw_value: Some(withdraw_value_value),
        from: Some(from_value),
        value: Some(value_value),
        delta: Some(delta_value),
        secret: Some(secret_value),
        leaf_index: Some(leaf_index_value),
        siblings: siblings_array.map(Some),
    };

    let public_inputs = circuit
        .public_inputs()
        .context("failed to compute public inputs")?;

    let mut rng = StdRng::seed_from_u64(42);
    let proof = Groth16::<Bn254>::prove(&params.pk, circuit.clone(), &mut rng)
        .context("failed to generate Groth16 proof")?;

    let verified = Groth16::<Bn254>::verify(&params.vk, &public_inputs, &proof)
        .context("failed to verify proof")?;
    if !verified {
        anyhow::bail!("generated Groth16 proof did not verify");
    }

    // 1) EVM-style calldata: proof + public inputs, as expected by the Solidity verifier
    let proof_calldata = [proof.to_eth(), public_inputs.to_eth()].concat();
    write_bytes(
        &output_dir.join(format!("{prefix}_groth16_proof_calldata.bin")),
        &proof_calldata,
    )?;

    // 2) Canonically serialized Groth16 proof (for Solana verifier / non-EVM consumers)
    let mut proof_bytes = Vec::new();
    proof
        .serialize_compressed(&mut proof_bytes)
        .context("failed to serialize Groth16 proof")?;
    write_bytes(
        &output_dir.join(format!("{prefix}_groth16_proof.bin")),
        &proof_bytes,
    )?;

    // 3) Canonically serialized public inputs (for non-EVM consumers like our future Solana verifier)
    let mut public_inputs_bytes = Vec::new();
    for input in &public_inputs {
        input
            .serialize_compressed(&mut public_inputs_bytes)
            .with_context(|| "failed to serialize public input")?;
    }
    write_bytes(
        &output_dir.join(format!("{prefix}_groth16_public_inputs.bin")),
        &public_inputs_bytes,
    )?;

    Ok(())
}

/// Local copy of `merkle_root_from_path` from `zkp::test_utils`, so this binary
/// does not depend on the test-only module.
fn merkle_root_from_path(
    config: &PoseidonConfig<Fr>,
    index: u64,
    leaf: Fr,
    siblings: &[Fr],
) -> Fr {
    use zkp::utils::poseidon::utils::circom_poseidon_hash;

    let mut current = leaf;
    for (depth, sibling) in siblings.iter().enumerate() {
        let bit = (index >> depth) & 1;
        let (left, right) = if bit == 0 {
            (current, *sibling)
        } else {
            (*sibling, current)
        };
        current = circom_poseidon_hash(config, &[left, right]);
    }
    current
}

fn write_bytes(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}
