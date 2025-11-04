use std::collections::HashMap;

use anyhow::Context;
use api_types::indexer::IndexedEvent;
use ark_bn254::Fr;
use zkp::{
    nova::constants::TRANSFER_TREE_HEIGHT,
    utils::{
        convertion::{address_to_fr, u256_to_fr},
        tree::{gadgets::leaf_hash::compute_leaf_hash, merkle_tree::MerkleProof},
    },
};

use crate::{
    indexer::IndexerClient, teleport::aggregation_tree::AggregationTreeState, tokens::TokenEntry,
};

#[derive(Clone, Debug)]
pub struct LocalTeleportMerkleProof {
    pub tree_index: u64,
    pub event: IndexedEvent,
    pub local_merkle_proof: MerkleProof,
}

pub async fn fetch_local_teleport_merkle_proofs(
    indexer: &dyn IndexerClient,
    token_entry: &TokenEntry,
    tree_index: u64,
    events: &[IndexedEvent],
) -> anyhow::Result<Vec<LocalTeleportMerkleProof>> {
    let leaf_indices: Vec<u64> = events.iter().map(|e| e.event_index).collect();
    let proofs = indexer
        .prove_many(
            token_entry.chain_id,
            token_entry.token_address,
            tree_index,
            &leaf_indices,
        )
        .await
        .context("Failed to fetch Merkle proofs from indexer")?;
    if proofs.len() != events.len() {
        anyhow::bail!(
            "Indexer returned {} proofs, but {} events were requested for tree index {}",
            proofs.len(),
            events.len(),
            tree_index
        );
    }
    let mut local_proofs = Vec::with_capacity(events.len());
    for (event, proof) in events.iter().zip(proofs.into_iter()) {
        if proof.leaf_index != event.event_index || proof.target_index != tree_index {
            anyhow::bail!(
                "Mismatch in proof indices for event index {}: proof leaf index = {}, proof target index = {}, expected target index = {}",
                event.event_index,
                proof.leaf_index,
                proof.target_index,
                tree_index
            );
        }
        let siblings: Vec<Fr> = proof.siblings.into_iter().map(u256_to_fr).collect();
        let merkle_proof = MerkleProof { siblings };
        // check proof
        let leaf_hash = leaf_hash(event);
        if merkle_proof.get_root(leaf_hash, event.event_index) != u256_to_fr(proof.root) {
            anyhow::bail!(
                "Invalid Merkle proof for event index {}: computed root = {}, proof root = {}",
                event.event_index,
                merkle_proof.get_root(leaf_hash, event.event_index),
                u256_to_fr(proof.root)
            );
        }
        local_proofs.push(LocalTeleportMerkleProof {
            tree_index,
            event: event.clone(),
            local_merkle_proof: merkle_proof,
        });
    }

    // sort by index
    local_proofs.sort_by_key(|w| w.event.event_index);

    Ok(local_proofs)
}

#[derive(Clone, Debug)]
pub struct GlobalTeleportMerkleProof {
    pub event: IndexedEvent,
    pub global_merkle_proof: MerkleProof,
    pub global_leaf_index: u64,
}

pub fn generate_global_teleport_merkle_proofs(
    aggregation_state: &AggregationTreeState,
    local_teleport_merkle_proofs: &HashMap<u64, Vec<LocalTeleportMerkleProof>>,
) -> anyhow::Result<Vec<GlobalTeleportMerkleProof>> {
    let mut global_proofs = Vec::new();
    for (chain_id, local_witness) in local_teleport_merkle_proofs.iter() {
        let aggregation_index = aggregation_state
            .chain_ids
            .iter()
            .position(|&id| id == *chain_id)
            .ok_or_else(|| {
                anyhow::anyhow!("Chain ID {} not found in aggregation state", chain_id)
            })? as u64;
        let aggregation_merkle_proof = aggregation_state.aggregation_tree.prove(aggregation_index);
        for witness in local_witness.iter() {
            // construct global merkle proof
            let global_merkle_proof = witness.local_merkle_proof.extend(&aggregation_merkle_proof);
            let global_leaf_index =
                (aggregation_index << TRANSFER_TREE_HEIGHT) + witness.event.event_index;

            // check merkle proof
            let leaf_hash = leaf_hash(&witness.event);
            let expected_root = global_merkle_proof.get_root(leaf_hash, global_leaf_index);
            if expected_root != u256_to_fr(aggregation_state.aggregation_root) {
                anyhow::bail!(
                    "Invalid global Merkle proof for global index {}: computed root = {}, expected root = {}",
                    global_leaf_index,
                    expected_root,
                    u256_to_fr(aggregation_state.aggregation_root)
                );
            }
            global_proofs.push(GlobalTeleportMerkleProof {
                event: witness.event.clone(),
                global_merkle_proof,
                global_leaf_index,
            });
        }
    }

    // sort by index
    global_proofs.sort_by_key(|w| w.global_leaf_index);

    Ok(global_proofs)
}

fn leaf_hash(event: &IndexedEvent) -> Fr {
    compute_leaf_hash(address_to_fr(event.to), u256_to_fr(event.value))
}
