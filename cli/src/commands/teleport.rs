use std::{collections::HashMap, path::Path, time::SystemTime};

use alloy::primitives::{Address, B256, U256};
use anyhow::{Context, Result, anyhow};
use client_common::{
    contracts::{
        gelato_relay::{
            self, RelayTaskState, RelayTeleportParams, encode_relay_single_teleport,
            encode_relay_teleport,
        },
        verifier::VerifierContract,
    },
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
    CommonArgs, RelayArgs, build_decider_client,
    commands::shared::{build_liquidity_manager, find_token_by_chain, format_tx_hash},
    proof::{batch::batch_teleport_proof, single::single_teleport_proof},
};

pub enum RedeemResult {
    AlreadyClaimed,
    NoProofs,
    Submitted,
}

/// Redeem eligible teleport transfers by generating the necessary proofs and submitting
/// the corresponding transactions.
#[allow(clippy::too_many_arguments)]
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

/// Redeem eligible teleport transfers via Gelato Relay (gasless).
#[allow(clippy::too_many_arguments)]
pub async fn redeem_transfers_via_relay(
    common_args: &CommonArgs,
    relay_args: &RelayArgs,
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

    let total_value = total_eligible_value;
    let recipient_hash = gr.to_u256();

    // Find token entry for the target chain
    let token_entry = find_token_by_chain(token_entries, gr.chain_id)?;
    let relay_address = token_entry.gelato_relay_address.ok_or_else(|| {
        anyhow!(
            "token '{}' is missing gelato_relay_address — relay mode not available for this chain",
            token_entry.label,
        )
    })?;

    // Build Merkle proofs (same as non-relay path)
    let mut local_teleport_mps = HashMap::new();
    for (chain_id, events_with_eligibility) in separated_events {
        let events = &events_with_eligibility.eligible;
        if events.is_empty() {
            continue;
        }
        let tree_index = aggregation_tree_state
            .get_tree_id_for_chain_id(*chain_id)
            .context(format!("no tree root index for chain id {}", chain_id))?;
        let entry = find_token_by_chain(token_entries, *chain_id)?;
        let local_proofs = fetch_local_teleport_merkle_proofs(indexer, entry, tree_index, events)
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

    // Generate ZK proof (single or batch)
    let (proof_bytes, is_single) = if global_merkle_proofs.len() == 1 {
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
        (single_proof, true)
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
        (batch_proof, false)
    };

    // Estimate relayer fee
    let liquidity_manager = build_liquidity_manager(token_entry)?;
    let fee_token = liquidity_manager
        .underlying_token()
        .await
        .context("failed to fetch underlying token address")?;

    println!("Estimating Gelato relay fee...");
    let fee_estimate = gelato_relay::estimate_relayer_fee(
        token_entry.chain_id,
        fee_token,
        None,
        &liquidity_manager,
    )
    .await
    .context("failed to estimate relayer fee")?;

    let relayer_fee = fee_estimate.relayer_fee;
    let max_fee = if let Some(cap) = relay_args.max_relay_fee {
        if cap < relayer_fee {
            anyhow::bail!(
                "estimated relayer fee {} exceeds --max-relay-fee cap {}",
                relayer_fee,
                cap
            );
        }
        cap
    } else {
        relayer_fee
    };

    println!("  Gelato gas fee : {}", fee_estimate.gelato_fee);
    println!("  Unwrap fee     : {}", fee_estimate.unwrap_fee);
    println!("  Total (w/ buf) : {}", relayer_fee);

    // EIP-712 signature
    let deadline = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .context("system time error")?
        .as_secs()
        + 3600; // 1 hour from now

    let domain_separator =
        gelato_relay::fetch_domain_separator(verifier.provider(), verifier.address())
            .await
            .context("failed to fetch EIP-712 domain separator from Verifier")?;

    let signature = gelato_relay::sign_relayer_fee_authorization(
        private_key,
        domain_separator,
        recipient_hash,
        total_value,
        max_fee,
        deadline,
    )
    .await
    .context("failed to sign relayer fee authorization")?;

    // Encode calldata
    let params = RelayTeleportParams {
        is_global: true,
        root_hint: aggregation_tree_state.latest_agg_seq,
        chain_id: gr.chain_id,
        recipient: gr.address,
        tweak: gr.tweak,
        proof: proof_bytes,
        relayer_fee,
        max_fee,
        deadline,
        signature,
        max_gelato_fee: fee_estimate.gelato_fee,
    };

    let calldata = if is_single {
        encode_relay_single_teleport(&params)
    } else {
        encode_relay_teleport(&params)
    };

    // Submit to Gelato
    println!("Submitting relay task to Gelato...");
    let task_id = gelato_relay::submit_relay_task(
        token_entry.chain_id,
        relay_address,
        &calldata,
        fee_token,
        relay_args.gelato_api_key.as_deref(),
        None,
    )
    .await
    .context("failed to submit relay task to Gelato")?;
    println!("Gelato task ID     : {}", task_id);

    // Poll for completion
    println!("Polling for task completion...");
    let result = gelato_relay::poll_relay_task(&task_id, None, None)
        .await
        .context("failed to poll relay task status")?;

    match result.task_state {
        RelayTaskState::ExecSuccess => {
            if let Some(tx_hash) = &result.transaction_hash {
                println!("Relay succeeded    : {}", tx_hash);
            } else {
                println!("Relay succeeded (no tx hash available)");
            }
        }
        RelayTaskState::ExecReverted => {
            let msg = result
                .last_check_message
                .as_deref()
                .unwrap_or("unknown reason");
            anyhow::bail!("Gelato relay task reverted: {}", msg);
        }
        RelayTaskState::Cancelled => {
            let msg = result
                .last_check_message
                .as_deref()
                .unwrap_or("unknown reason");
            anyhow::bail!("Gelato relay task cancelled: {}", msg);
        }
        _ => {
            let msg = result.last_check_message.as_deref().unwrap_or("timed out");
            println!(
                "Relay task still pending after polling: {} — check Gelato status for task {}",
                msg, task_id
            );
        }
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
