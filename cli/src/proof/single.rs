use alloy::primitives::U256;
use anyhow::Context as _;
use ark_bn254::Fr;
use ark_ff::fields::AdditiveGroup;
use client_common::indexer::IndexedEvent;
use rand::rngs::OsRng;
use std::{fs, path::Path};
use zkp::{
    groth16::{params::Groth16Params, withdraw::SingleWithdrawCircuit},
    nova::constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
    utils::{
        convertion::u256_to_fr, poseidon::utils::circom_poseidon_config,
        tree::merkle_tree::MerkleProof,
    },
};

pub fn single_teleport_proof<const DEPTH: usize>(
    artifacts_dir: &Path,
    recipient: Fr,
    merkle_root: U256,
    event: IndexedEvent,
    merkle_proof: MerkleProof,
    leaf_index: u64,
    secret: Fr,
) -> anyhow::Result<Vec<u8>> {
    let withdraw_params = load_single_withdraw_params(artifacts_dir, DEPTH)
        .context("failed to load single withdraw Groth16 params")?;
    let poseidon_params = circom_poseidon_config();
    let merkle_root = u256_to_fr(merkle_root);
    let value = u256_to_fr(event.value);
    let siblings: [Option<Fr>; DEPTH] = merkle_proof
        .siblings
        .into_iter()
        .map(|s| Some(s))
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid number of siblings in global Merkle proof"))?;
    let circuit = SingleWithdrawCircuit::<Fr, DEPTH> {
        poseidon_params,
        merkle_root: Some(merkle_root),
        recipient: Some(recipient),
        withdraw_value: Some(value),
        value: Some(value),
        delta: Some(Fr::ZERO),
        secret: Some(secret),
        leaf_index: Some(leaf_index),
        siblings,
    };
    let public_inputs = circuit.public_inputs()?;

    log::info!("Start Groth16 proof generation for single withdraw",);
    let proof = withdraw_params
        .generate_proof(&mut OsRng, circuit, &public_inputs)
        .context("failed to create single global teleport Groth16 proof")?;
    log::info!("Single withdraw Groth16 proof generated");
    Ok(proof)
}

pub fn load_single_withdraw_params(
    artifacts_dir: &Path,
    depth: usize,
) -> anyhow::Result<Groth16Params> {
    let prefix = match depth {
        TRANSFER_TREE_HEIGHT => "withdraw_local",
        GLOBAL_TRANSFER_TREE_HEIGHT => "withdraw_global",
        _ => {
            anyhow::bail!("Unsupported transfer tree depth: {}", depth)
        }
    };
    let pk = fs::read(artifacts_dir.join(format!("{}_groth16_pk.bin", prefix)))
        .with_context(|| format!("failed to read {}_groth16_pp.bin", prefix))?;
    let vk = fs::read(artifacts_dir.join(format!("{}_groth16_vk.bin", prefix)))
        .with_context(|| format!("failed to read {}_groth16_vp.bin", prefix))?;
    let params = Groth16Params::from_bytes(pk, vk)
        .with_context(|| format!("failed to parse {} Groth16 params", prefix))?;
    Ok(params)
}
