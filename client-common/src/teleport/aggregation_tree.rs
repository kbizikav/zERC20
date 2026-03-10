use alloy::primitives::U256;
use anyhow::Context;
use zkp::{
    nova::constants::AGGREGATION_TREE_HEIGHT,
    utils::{convertion::u256_to_fr, tree::merkle_tree::MerkleTree},
};

use crate::contracts::{
    hub::{AggregationRootUpdatedEvent, HubContract},
    verifier::VerifierContract,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferRootScope {
    Global,
    Local,
}

#[derive(Clone, Debug)]
pub struct AggregationTreeState {
    pub scope: TransferRootScope,
    pub root_hint: u64,
    pub aggregation_root: U256,
    pub aggregation_tree: MerkleTree,
    pub tree_root_indices: Vec<u64>,
    pub chain_ids: Vec<u64>,
    pub snapshot: Vec<U256>,
}

impl AggregationTreeState {
    pub fn get_tree_id_for_chain_id(&self, chain_id: u64) -> Option<u64> {
        let aggregation_index = self.chain_ids.iter().position(|&id| id == chain_id);
        aggregation_index.map(|idx| self.tree_root_indices[idx])
    }
}

pub async fn fetch_aggregation_tree_state(
    event_block_span: u64,
    verifier: &VerifierContract,
    hub: &HubContract,
    chain_id: u64,
    use_local_root: bool,
) -> anyhow::Result<AggregationTreeState> {
    if use_local_root {
        return fetch_local_aggregation_tree_state(verifier, chain_id).await;
    }

    let latest_agg_seq = verifier
        .latest_agg_seq()
        .await
        .context("Failed to fetch latest aggregation sequence from verifier")?;
    if latest_agg_seq == 0 {
        anyhow::bail!("No aggregation reached yet (latest_agg_seq is 0)");
    }
    let onchain_global_root = verifier
        .global_transfer_root(latest_agg_seq)
        .await
        .context("Failed to fetch global transfer root from verifier")?;
    let aggregation_event = find_aggregation_event(hub, latest_agg_seq, event_block_span).await?;
    if aggregation_event.root != onchain_global_root {
        anyhow::bail!(
            "Mismatch in global transfer root for aggregation sequence {}: on-chain root = {}, event root = {}",
            latest_agg_seq,
            onchain_global_root,
            aggregation_event.root
        );
    }
    let mut aggregation_tree = MerkleTree::new(AGGREGATION_TREE_HEIGHT);
    for (idx, &root) in aggregation_event.snapshot.iter().enumerate() {
        if root != U256::ZERO {
            aggregation_tree.update_leaf(idx as u64, u256_to_fr(root));
        }
    }
    if aggregation_tree.get_root() != u256_to_fr(aggregation_event.root) {
        anyhow::bail!(
            "Aggregation tree root mismatch: computed root = {}, event root = {}",
            aggregation_tree.get_root(),
            aggregation_event.root
        );
    }
    // get hub token info
    let token_infos = hub
        .token_infos()
        .await
        .context("Failed to fetch token infos from hub contract")?;
    let chain_ids = token_infos.into_iter().map(|info| info.chain_id).collect();

    Ok(AggregationTreeState {
        scope: TransferRootScope::Global,
        root_hint: latest_agg_seq,
        aggregation_root: aggregation_event.root,
        aggregation_tree,
        tree_root_indices: aggregation_event.transfer_tree_indices,
        chain_ids,
        snapshot: aggregation_event.snapshot,
    })
}

async fn fetch_local_aggregation_tree_state(
    verifier: &VerifierContract,
    chain_id: u64,
) -> anyhow::Result<AggregationTreeState> {
    let latest_proved_index = verifier
        .latest_proved_index()
        .await
        .context("Failed to fetch latest proved index from verifier")?;
    if latest_proved_index == 0 {
        anyhow::bail!("No proved local transfer root available yet (latest_proved_index is 0)");
    }

    let local_root = verifier
        .proved_transfer_root(latest_proved_index)
        .await
        .context("Failed to fetch local proved transfer root from verifier")?;
    if local_root == U256::ZERO {
        anyhow::bail!(
            "Local proved transfer root at index {} is zero",
            latest_proved_index
        );
    }

    Ok(AggregationTreeState {
        scope: TransferRootScope::Local,
        root_hint: latest_proved_index,
        aggregation_root: local_root,
        aggregation_tree: MerkleTree::new(AGGREGATION_TREE_HEIGHT),
        tree_root_indices: vec![latest_proved_index],
        chain_ids: vec![chain_id],
        snapshot: Vec::new(),
    })
}

async fn find_aggregation_event(
    hub: &HubContract,
    target_seq: u64,
    block_span: u64,
) -> anyhow::Result<AggregationRootUpdatedEvent> {
    if block_span == 0 {
        anyhow::bail!("event_block_span must be greater than zero");
    }
    let latest_block = hub
        .latest_block()
        .await
        .context("Failed to fetch latest block from hub contract")?;
    let mut to_block = latest_block;
    loop {
        let from_block = (to_block + 1).saturating_sub(block_span);
        let events = hub
            .aggregation_root_events(from_block, to_block)
            .await
            .context("Failed to fetch aggregation root events from hub contract")?;
        for event in events.into_iter().rev() {
            if event.agg_seq == target_seq {
                return Ok(event);
            }
        }
        if from_block == 0 {
            break;
        }
        to_block = from_block.saturating_sub(1);
    }

    anyhow::bail!("Aggregation event with sequence {} not found", target_seq);
}
