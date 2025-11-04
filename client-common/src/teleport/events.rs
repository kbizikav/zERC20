use std::collections::HashMap;

use alloy::primitives::{Address, U256};
use anyhow::Context;
use api_types::indexer::IndexedEvent;

use crate::{
    contracts::z_erc20::ZErc20Contract, indexer::IndexerClient,
    teleport::aggregation_tree::AggregationTreeState, tokens::TokenEntry,
};

pub async fn fetch_transfer_events(
    indexer: &dyn IndexerClient,
    indexer_fetch_limit: Option<usize>,
    token_entries: &[TokenEntry],
    token_clients: &[ZErc20Contract],
    burn_addresses: &[Address],
) -> anyhow::Result<HashMap<u64, Vec<IndexedEvent>>> {
    if token_entries.len() != token_clients.len() {
        anyhow::bail!(
            "token entries length {} does not match token clients length {}",
            token_entries.len(),
            token_clients.len()
        );
    }

    // First, filter burn addresses with non-zero balances for each token using the token clients.
    let mut nonzero_balance_addresses: HashMap<u64, Vec<Address>> = HashMap::new();
    for address in burn_addresses {
        for (token_entry, token_client) in token_entries.iter().zip(token_clients) {
            let balance = token_client.balance_of(*address).await.context(format!(
                "failed to fetch balance at chain_id: {}",
                token_entry.chain_id
            ))?;
            if !balance.is_zero() {
                nonzero_balance_addresses
                    .entry(token_entry.chain_id)
                    .or_default()
                    .push(*address);
            }
        }
    }

    // Next, fetch transfer events for the filtered addresses from the indexer.
    let mut all_events: HashMap<u64, Vec<IndexedEvent>> = HashMap::new();
    for (chain_id, addresses) in nonzero_balance_addresses {
        let token_entry = token_entries
            .iter()
            .find(|entry| entry.chain_id == chain_id)
            .context(format!("token entry not found for chain_id: {}", chain_id))?;
        let mut events = Vec::new();
        for address in &addresses {
            let events_of_address = indexer
                .events_by_recipient(
                    token_entry.chain_id,
                    token_entry.token_address,
                    *address,
                    indexer_fetch_limit.clone(),
                )
                .await?;
            events.extend(events_of_address);
        }
        all_events.insert(chain_id, events);
    }

    Ok(all_events)
}

pub struct EventsWithEligibility {
    pub eligible: Vec<IndexedEvent>,
    pub ineligible: Vec<IndexedEvent>,
}

impl EventsWithEligibility {
    pub fn eligible_total_value(&self) -> U256 {
        self.eligible
            .iter()
            .fold(U256::ZERO, |acc, event| acc + event.value)
    }

    pub fn ineligible_total_value(&self) -> U256 {
        self.ineligible
            .iter()
            .fold(U256::ZERO, |acc, event| acc + event.value)
    }
}

pub fn separate_events_by_eligibility(
    aggregation_tree_state: &AggregationTreeState,
    events: &HashMap<u64, Vec<IndexedEvent>>,
) -> anyhow::Result<HashMap<u64, EventsWithEligibility>> {
    let mut separated_events: HashMap<u64, EventsWithEligibility> = HashMap::new();
    for (chain_id, event_list) in events {
        let tree_root_index = aggregation_tree_state
            .get_tree_id_for_chain_id(*chain_id)
            .context(format!("no tree root index for chain id {}", chain_id))?;
        let (eligible, ineligible): (Vec<IndexedEvent>, Vec<IndexedEvent>) = event_list
            .iter()
            .cloned()
            .partition(|event| event.event_index < tree_root_index);
        separated_events.insert(
            *chain_id,
            EventsWithEligibility {
                eligible,
                ineligible,
            },
        );
    }
    Ok(separated_events)
}
