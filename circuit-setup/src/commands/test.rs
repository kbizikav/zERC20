// SPDX-License-Identifier: BUSL-1.1

use std::convert::TryInto;
use std::fs;
use std::path::Path;
use std::time::Instant;

use alloy::primitives::U256;
use anyhow::{Context, Result};
use ark_bn254::Fr;
use ark_crypto_primitives::sponge::poseidon::PoseidonConfig;
use ark_ff::Zero;
use folding_schemes::FoldingScheme;
use rand::{rngs::StdRng, SeedableRng};

use zerc20_zkp::{
    circuits::burn_address::{compute_burn_address_from_secret, find_pow_nonce, secret_from_nonce},
    groth16::{params::Groth16Params, withdraw::SingleWithdrawCircuit},
    nova::{
        constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
        params::{DeciderParams, FParams, NovaParams},
        root_nova::{RootCircuit, RootExternalInputs},
        withdraw_nova::{dummy_withdraw_ext_input, WithdrawCircuit, WITHDRAW_STATE_LEN},
    },
    utils::{
        poseidon::utils::{circom_poseidon2_config, circom_poseidon3_config},
        tree::{gadgets::leaf_hash::compute_leaf_hash, merkle_tree::MerkleProof},
    },
};

/// Test all circuit artifacts by generating and verifying dummy proofs.
pub fn test_artifacts(artifacts_dir: &Path) -> Result<()> {
    let poseidon2_config = circom_poseidon2_config::<Fr>();
    let poseidon3_config = circom_poseidon3_config();

    println!();
    println!("=== Testing Circuit Artifacts ===");
    println!();

    // Test root circuit
    println!("[1/5] Testing root circuit...");
    test_root_circuit(
        artifacts_dir,
        (poseidon2_config.clone(), poseidon3_config.clone()),
    )?;
    println!("  Root circuit: OK");

    // Test withdraw_local nova circuit
    println!("[2/5] Testing withdraw_local nova circuit...");
    test_withdraw_nova_circuit::<TRANSFER_TREE_HEIGHT>(
        "withdraw_local",
        artifacts_dir,
        (poseidon2_config.clone(), poseidon3_config.clone()),
    )?;
    println!("  Withdraw local nova: OK");

    // Test withdraw_global nova circuit
    println!("[3/5] Testing withdraw_global nova circuit...");
    test_withdraw_nova_circuit::<GLOBAL_TRANSFER_TREE_HEIGHT>(
        "withdraw_global",
        artifacts_dir,
        (poseidon2_config.clone(), poseidon3_config.clone()),
    )?;
    println!("  Withdraw global nova: OK");

    // Test withdraw_local groth16 circuit
    println!("[4/5] Testing withdraw_local groth16 circuit...");
    test_groth16_circuit::<TRANSFER_TREE_HEIGHT>(
        "withdraw_local",
        artifacts_dir,
        &poseidon2_config,
        &poseidon3_config,
    )?;
    println!("  Withdraw local groth16: OK");

    // Test withdraw_global groth16 circuit
    println!("[5/5] Testing withdraw_global groth16 circuit...");
    test_groth16_circuit::<GLOBAL_TRANSFER_TREE_HEIGHT>(
        "withdraw_global",
        artifacts_dir,
        &poseidon2_config,
        &poseidon3_config,
    )?;
    println!("  Withdraw global groth16: OK");

    println!();
    println!("=== All Tests Passed ===");
    println!();

    Ok(())
}

fn test_root_circuit(artifacts_dir: &Path, f_params: FParams<RootCircuit<Fr>>) -> Result<()> {
    // Load params
    let nova_pp = fs::read(artifacts_dir.join("root_nova_pp.bin"))
        .context("failed to read root_nova_pp.bin")?;
    let nova_vp = fs::read(artifacts_dir.join("root_nova_vp.bin"))
        .context("failed to read root_nova_vp.bin")?;
    let decider_pp = fs::read(artifacts_dir.join("root_decider_pp.bin"))
        .context("failed to read root_decider_pp.bin")?;
    let decider_vp = fs::read(artifacts_dir.join("root_decider_vp.bin"))
        .context("failed to read root_decider_vp.bin")?;

    let nova_params = NovaParams::<RootCircuit<Fr>>::from_bytes(f_params, nova_pp, nova_vp)
        .context("failed to deserialize root nova params")?;
    let decider_params = DeciderParams::<RootCircuit<Fr>>::from_bytes(decider_pp, decider_vp)
        .context("failed to deserialize root decider params")?;

    let state_len = nova_params.state_len()?;

    // Initialize nova
    let mut nova = nova_params
        .initial_nova(vec![Fr::zero(); state_len])
        .context("failed to initialize root nova")?;

    // Create dummy input and prove steps
    let mut rng = StdRng::seed_from_u64(0xDEAD_BEEF);
    let external_input = RootExternalInputs::<Fr> {
        is_dummy: true,
        from_address: Fr::zero(),
        to_address: Fr::zero(),
        value: Fr::zero(),
        siblings: [Fr::zero(); TRANSFER_TREE_HEIGHT],
    };

    let start = Instant::now();
    nova.prove_step(&mut rng, external_input.clone(), None)
        .context("failed to prove root nova step 1")?;
    nova.prove_step(&mut rng, external_input, None)
        .context("failed to prove root nova step 2")?;
    println!("    Nova steps: {:.2}s", start.elapsed().as_secs_f64());

    // Generate decider proof (internally proves and verifies)
    let start = Instant::now();
    let proof = decider_params
        .generate_decider_proof(nova)
        .context("failed to generate/verify root decider proof")?;
    println!(
        "    Decider proof (generated + verified): {:.2}s ({} bytes)",
        start.elapsed().as_secs_f64(),
        proof.len()
    );

    Ok(())
}

fn test_withdraw_nova_circuit<const DEPTH: usize>(
    prefix: &str,
    artifacts_dir: &Path,
    f_params: FParams<WithdrawCircuit<Fr, DEPTH>>,
) -> Result<()> {
    // Load params
    let nova_pp = fs::read(artifacts_dir.join(format!("{}_nova_pp.bin", prefix)))
        .with_context(|| format!("failed to read {}_nova_pp.bin", prefix))?;
    let nova_vp = fs::read(artifacts_dir.join(format!("{}_nova_vp.bin", prefix)))
        .with_context(|| format!("failed to read {}_nova_vp.bin", prefix))?;
    let decider_pp = fs::read(artifacts_dir.join(format!("{}_decider_pp.bin", prefix)))
        .with_context(|| format!("failed to read {}_decider_pp.bin", prefix))?;
    let decider_vp = fs::read(artifacts_dir.join(format!("{}_decider_vp.bin", prefix)))
        .with_context(|| format!("failed to read {}_decider_vp.bin", prefix))?;

    let nova_params =
        NovaParams::<WithdrawCircuit<Fr, DEPTH>>::from_bytes(f_params, nova_pp, nova_vp)
            .with_context(|| format!("failed to deserialize {} nova params", prefix))?;
    let decider_params =
        DeciderParams::<WithdrawCircuit<Fr, DEPTH>>::from_bytes(decider_pp, decider_vp)
            .with_context(|| format!("failed to deserialize {} decider params", prefix))?;

    // Initialize nova
    let mut nova = nova_params
        .initial_nova(vec![Fr::zero(); WITHDRAW_STATE_LEN])
        .with_context(|| format!("failed to initialize {} nova", prefix))?;

    // Create dummy input and prove steps
    let mut rng = StdRng::seed_from_u64(0xCAFE_BABE);
    let external_input = dummy_withdraw_ext_input::<DEPTH>(1, U256::ZERO);

    let start = Instant::now();
    nova.prove_step(&mut rng, external_input, None)
        .with_context(|| format!("failed to prove {} nova step 1", prefix))?;
    let external_input = dummy_withdraw_ext_input::<DEPTH>(2, U256::ZERO);
    nova.prove_step(&mut rng, external_input, None)
        .with_context(|| format!("failed to prove {} nova step 2", prefix))?;
    println!("    Nova steps: {:.2}s", start.elapsed().as_secs_f64());

    // Generate decider proof (internally proves and verifies)
    let start = Instant::now();
    let proof = decider_params
        .generate_decider_proof(nova)
        .with_context(|| format!("failed to generate/verify {} decider proof", prefix))?;
    println!(
        "    Decider proof (generated + verified): {:.2}s ({} bytes)",
        start.elapsed().as_secs_f64(),
        proof.len()
    );

    Ok(())
}

fn test_groth16_circuit<const DEPTH: usize>(
    prefix: &str,
    artifacts_dir: &Path,
    poseidon2_config: &PoseidonConfig<Fr>,
    poseidon3_config: &PoseidonConfig<Fr>,
) -> Result<()> {
    // Load params
    let pk = fs::read(artifacts_dir.join(format!("{}_groth16_pk.bin", prefix)))
        .with_context(|| format!("failed to read {}_groth16_pk.bin", prefix))?;
    let vk = fs::read(artifacts_dir.join(format!("{}_groth16_vk.bin", prefix)))
        .with_context(|| format!("failed to read {}_groth16_vk.bin", prefix))?;

    let params = Groth16Params::from_bytes(pk, vk)
        .with_context(|| format!("failed to deserialize {} groth16 params", prefix))?;

    // Build test circuit
    let circuit = build_test_withdraw_circuit::<DEPTH>(poseidon2_config, poseidon3_config);
    let public_inputs = circuit
        .public_inputs()
        .with_context(|| format!("failed to get {} public inputs", prefix))?;

    // Generate and verify groth16 proof (internally proves and verifies)
    let mut rng = StdRng::seed_from_u64(0xFEED_FACE);
    let start = Instant::now();
    let proof = params
        .generate_proof(&mut rng, circuit, &public_inputs)
        .with_context(|| format!("failed to generate/verify {} groth16 proof", prefix))?;
    println!(
        "    Groth16 proof (generated + verified): {:.2}s ({} bytes)",
        start.elapsed().as_secs_f64(),
        proof.len()
    );

    Ok(())
}

fn build_test_withdraw_circuit<const DEPTH: usize>(
    poseidon2_config: &PoseidonConfig<Fr>,
    poseidon3_config: &PoseidonConfig<Fr>,
) -> SingleWithdrawCircuit<Fr, DEPTH> {
    let recipient = Fr::from(321u64);
    let secret_seed = Fr::from(654u64);
    let nonce = find_pow_nonce(recipient, secret_seed);
    let secret = secret_from_nonce(secret_seed, nonce);
    let address =
        compute_burn_address_from_secret(recipient, secret).expect("nonce should satisfy PoW");
    let from = Fr::from(777u64);
    let value = Fr::from(1_000u64);
    let delta = Fr::from(123u64);
    let leaf_index: u64 = 5;
    let withdraw_value = value - delta;

    let leaf = compute_leaf_hash(from, address, value);
    let proof = MerkleProof::dummy(DEPTH);
    let siblings: [Fr; DEPTH] = proof
        .siblings
        .clone()
        .try_into()
        .expect("dummy proof length matches depth");
    let merkle_root = proof.get_root(leaf, leaf_index);

    SingleWithdrawCircuit {
        poseidon2_params: poseidon2_config.clone(),
        poseidon3_params: poseidon3_config.clone(),
        merkle_root: Some(merkle_root),
        recipient: Some(recipient),
        withdraw_value: Some(withdraw_value),
        from: Some(from),
        value: Some(value),
        delta: Some(delta),
        secret: Some(secret),
        leaf_index: Some(leaf_index),
        siblings: siblings.map(Some),
    }
}
