use std::{collections::HashMap, path::Path};

use alloy::primitives::{Address, B256, U256};
use anyhow::{Context, Result};
use client_common::{
    contracts::verifier::VerifierContract,
    indexer::{HttpIndexerClient, IndexedEvent},
    teleport::{
        aggregation_tree::AggregationTreeState,
        events::EventsWithEligibility,
        merkle_proofs::{
            fetch_local_teleport_merkle_proofs, generate_global_teleport_merkle_proofs,
        },
    },
    tokens::TokenEntry,
};
use zkp::{
    nova::constants::GLOBAL_TRANSFER_TREE_HEIGHT,
    utils::{convertion::b256_to_fr, general_recipient::GeneralRecipient},
};

use crate::{
    CommonArgs, build_decider_client,
    commands::shared::{find_token_by_chain, format_tx_hash},
    proof::{batch::batch_teleport_proof, single::single_teleport_proof},
};

pub enum RedeemResult {
    AlreadyClaimed,
    NoProofs,
    Submitted,
}

/// Redeem eligible teleport transfers by generating the necessary proofs and submitting
/// the corresponding transactions.
pub async fn redeem_transfers(
    common_args: &CommonArgs,
    verifier: &VerifierContract,
    indexer: &HttpIndexerClient,
    aggregation_tree_state: &AggregationTreeState,
    separated_events: &HashMap<u64, EventsWithEligibility>,
    burn_address_to_secret: &HashMap<Address, B256>,
    gr: GeneralRecipient,
    token_entries: &[TokenEntry],
    private_key: B256,
    artifacts_dir: &Path,
) -> Result<RedeemResult> {
    let total_eligible_value = separated_events
        .values()
        .map(|events| events.eligible_total_value())
        .sum::<U256>();

    let total_teleported = verifier
        .total_teleported(gr.to_u256())
        .await
        .context("failed to fetch total teleported amount")?;
    if total_eligible_value <= total_teleported {
        return Ok(RedeemResult::AlreadyClaimed);
    }

    let mut local_teleport_mps = HashMap::new();
    for (chain_id, events_with_eligibility) in separated_events {
        let events = &events_with_eligibility.eligible;
        if events.is_empty() {
            continue;
        }
        let tree_index = aggregation_tree_state
            .get_tree_id_for_chain_id(*chain_id)
            .context(format!("no tree root index for chain id {}", chain_id))?;
        let token_entry = find_token_by_chain(token_entries, *chain_id)?;
        let local_proofs =
            fetch_local_teleport_merkle_proofs(indexer, token_entry, tree_index, events)
                .await
                .context("failed to fetch local teleport Merkle proofs")?;
        local_teleport_mps.insert(*chain_id, local_proofs);
    }
    let global_merkle_proofs =
        generate_global_teleport_merkle_proofs(aggregation_tree_state, &local_teleport_mps)
            .context("failed to generate global teleport Merkle proofs")?;

    if global_merkle_proofs.is_empty() {
        return Ok(RedeemResult::NoProofs);
    }

    if global_merkle_proofs.len() == 1 {
        let global_proof = &global_merkle_proofs[0];
        let secret = burn_address_to_secret
            .get(&global_proof.event.to)
            .context("missing secret for burn address")?;
        let single_proof = single_teleport_proof::<GLOBAL_TRANSFER_TREE_HEIGHT>(
            artifacts_dir,
            gr.to_fr(),
            aggregation_tree_state.aggregation_root,
            global_proof.event.clone(),
            global_proof.global_merkle_proof.clone(),
            global_proof.global_leaf_index,
            b256_to_fr(*secret),
        )
        .context("failed to generate single teleport proof")?;
        let pending = verifier
            .single_teleport(
                private_key,
                true,
                aggregation_tree_state.latest_agg_seq,
                gr,
                &single_proof,
            )
            .await
            .context("failed to submit single global teleport transaction")?;
        let tx_hash = format_tx_hash(pending.tx_hash().as_slice());
        println!("Submitted teleport  : {}", tx_hash);
    } else {
        let mut events = Vec::new();
        let mut merkle_proofs = Vec::new();
        let mut leaf_indices = Vec::new();
        let mut secrets = Vec::new();
        for global_proof in &global_merkle_proofs {
            events.push(global_proof.event.clone());
            merkle_proofs.push(global_proof.global_merkle_proof.clone());
            leaf_indices.push(global_proof.global_leaf_index);
            let secret = burn_address_to_secret
                .get(&global_proof.event.to)
                .context("missing secret for burn address")?;
            secrets.push(b256_to_fr(*secret));
        }
        let decider = build_decider_client(common_args, "teleport redemption")?;
        let batch_proof = batch_teleport_proof::<GLOBAL_TRANSFER_TREE_HEIGHT>(
            artifacts_dir,
            &decider,
            gr.to_fr(),
            aggregation_tree_state.aggregation_root,
            &events,
            &merkle_proofs,
            &leaf_indices,
            &secrets,
        )
        .await
        .context("failed to generate batch teleport proof")?;

        let pending = verifier
            .teleport(
                private_key,
                true,
                aggregation_tree_state.latest_agg_seq,
                gr,
                &batch_proof,
            )
            .await
            .context("failed to submit batch global teleport transaction")?;
        let tx_hash = format_tx_hash(pending.tx_hash().as_slice());
        println!("Submitted teleport  : {}", tx_hash);
    }

    Ok(RedeemResult::Submitted)
}

pub fn print_events(chain_id: u64, events: &EventsWithEligibility) {
    println!("Chain ID {}:", chain_id);
    println!(
        "  Eligible   : total {:>3} events, total value {}",
        events.eligible.len(),
        events.eligible_total_value()
    );
    for event in &events.eligible {
        print_event_line("✅", event);
    }

    println!(
        "  Pending    : total {:>3} events, total value {}",
        events.ineligible.len(),
        events.ineligible_total_value()
    );
    for event in &events.ineligible {
        print_event_line("⏳", event);
    }
}

fn print_event_line(prefix: &str, event: &IndexedEvent) {
    println!(
        "    {} index {:>5} | from {} | to {} | value {}",
        prefix, event.event_index, event.from, event.to, event.value
    );
}
