use alloy::primitives::U256;
use anyhow::{Context as _, anyhow};
use api_types::prover::CircuitKind;
use ark_bn254::Fr;
use ark_ff::AdditiveGroup;
use ark_ff::Zero;
use ark_serialize::CanonicalSerialize;
use client_common::{indexer::IndexedEvent, prover::DeciderClient};
use folding_schemes::FoldingScheme;
use rand::Rng;
use std::{fs, path::Path};
use zkp::nova::constants::GLOBAL_TRANSFER_TREE_HEIGHT;
use zkp::nova::constants::TRANSFER_TREE_HEIGHT;
use zkp::{
    nova::{
        params::NovaParams,
        withdraw_nova::{WithdrawCircuit, WithdrawExternalInputs, dummy_withdraw_ext_input},
    },
    utils::{
        convertion::u256_to_fr, poseidon::utils::circom_poseidon_config,
        tree::merkle_tree::MerkleProof,
    },
};

use crate::commands::invoice::NUM_BATCH_INVOICES;

pub async fn batch_teleport_proof<const DEPTH: usize>(
    artifacts_dir: &Path,
    decider: &dyn DeciderClient,
    recipient: Fr,
    merkle_root: U256,
    events: &[IndexedEvent],
    merkle_proofs: &[MerkleProof],
    leaf_indices: &[u64],
    secrets: &[Fr],
) -> anyhow::Result<Vec<u8>> {
    if events.len() != merkle_proofs.len()
        || events.len() != leaf_indices.len()
        || events.len() != secrets.len()
    {
        anyhow::bail!(
            "Mismatched lengths: events {}, merkle_proofs {}, leaf_indices {}, secrets {}",
            events.len(),
            merkle_proofs.len(),
            leaf_indices.len(),
            secrets.len()
        );
    }

    let nova_params = load_withdraw_params::<DEPTH>(artifacts_dir)
        .context("failed to load batch withdraw Nova params")?;

    let mut external_inputs = Vec::new();

    for i in 0..events.len() {
        let event = &events[i];
        let merkle_proof = &merkle_proofs[i];
        let leaf_index = leaf_indices[i];
        let secret = secrets[i];
        let siblings: [Fr; DEPTH] =
            merkle_proof.siblings.clone().try_into().map_err(|_| {
                anyhow::anyhow!("invalid number of siblings in global Merkle proof")
            })?;
        external_inputs.push(WithdrawExternalInputs::<Fr, DEPTH> {
            is_dummy: Fr::ZERO,
            value: u256_to_fr(event.value),
            secret,
            leaf_index: Fr::from(leaf_index),
            siblings,
        });
    }

    // add dummy steps
    let mut rng = rand::thread_rng();
    let num_dummy_steps = rng.gen_range(1..NUM_BATCH_INVOICES);
    let offset = (1u64 << DEPTH) - 1 - num_dummy_steps as u64;
    for i in 0..num_dummy_steps {
        let index = offset + i as u64;
        let dummy_input = dummy_withdraw_ext_input::<DEPTH>(index, U256::ZERO);
        external_inputs.push(dummy_input);
    }

    log::info!(
        "Start IVC proof generation for batch withdraw with {} events and {} dummy steps (total {})",
        events.len(),
        num_dummy_steps,
        external_inputs.len()
    );
    let mut nova = nova_params
        .initial_nova(initial_state(u256_to_fr(merkle_root), recipient))
        .context("failed to initialize batch withdraw Nova")?;

    for external_input in external_inputs {
        nova.prove_step(&mut rng, external_input, None)
            .context("failed to prove step in batch withdraw Nova")?;
    }
    let ivc_proof = nova.ivc_proof();
    nova_params
        .verify(ivc_proof.clone())
        .context("failed to verify batch withdraw Nova proof")?;
    log::info!("Batch withdraw Nova proof generated and verified");

    let circuit_kind = match DEPTH {
        TRANSFER_TREE_HEIGHT => CircuitKind::WithdrawLocal,
        GLOBAL_TRANSFER_TREE_HEIGHT => CircuitKind::WithdrawGlobal,
        _ => {
            return Err(anyhow!(
                "unsupported circuit depth for batch withdraw proof: {}",
                DEPTH
            ));
        }
    };
    let mut proof_bytes = Vec::new();
    ivc_proof
        .serialize_uncompressed(&mut proof_bytes)
        .context("failed to serialize batch withdraw Nova proof")?;

    log::info!("Producing decider proof for batch withdraw...");
    let decider_proof = decider
        .produce_decider_proof(circuit_kind, &proof_bytes)
        .await
        .context("failed to produce decider proof for batch withdraw")?;
    log::info!("Decider proof for batch withdraw produced");
    Ok(decider_proof)
}

pub fn load_withdraw_params<const DEPTH: usize>(
    artifacts_dir: &Path,
) -> anyhow::Result<NovaParams<WithdrawCircuit<Fr, DEPTH>>> {
    let prefix = match DEPTH {
        TRANSFER_TREE_HEIGHT => "withdraw_local",
        GLOBAL_TRANSFER_TREE_HEIGHT => "withdraw_global",
        _ => {
            anyhow::bail!("Unsupported transfer tree depth: {}", DEPTH)
        }
    };
    let poseidon_params = circom_poseidon_config::<Fr>();
    let pp = fs::read(artifacts_dir.join(format!("{}_nova_pp.bin", prefix)))
        .with_context(|| format!("failed to read {}_nova_pp.bin", prefix))?;
    let vp = fs::read(artifacts_dir.join(format!("{}_nova_vp.bin", prefix)))
        .with_context(|| format!("failed to read {}_nova_vp.bin", prefix))?;
    NovaParams::from_bytes(poseidon_params, pp, vp)
        .map_err(|err| anyhow!("failed to deserialize withdraw nova params: {}", err))
}

fn initial_state(root: Fr, recipient: Fr) -> Vec<Fr> {
    vec![root, recipient, Fr::zero(), Fr::zero()]
}
