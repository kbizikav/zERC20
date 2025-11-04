use std::{collections::HashMap, fs, path::Path};

use alloy::primitives::{Address, B256};
use anyhow::{Context, Result, anyhow};
use client_common::{
    payment::burn_address::FullBurnAddress,
    teleport::{
        aggregation_tree::fetch_aggregation_tree_state,
        events::{fetch_transfer_events, separate_events_by_eligibility},
    },
    tokens::{HubEntry, TokenEntry},
};
use hex;

use crate::{
    CommonArgs, ReceiveTransferArgs, build_indexer_client,
    commands::{
        scan_receive_transfers::ScannedTransfer,
        shared::{build_erc20, build_hub, build_verifier, find_token_by_chain},
        teleport::{RedeemResult, print_events, redeem_transfers},
    },
};

pub async fn run(
    common_args: &CommonArgs,
    args: &ReceiveTransferArgs,
    tokens: &[TokenEntry],
    hub: Option<&HubEntry>,
    private_key: B256,
) -> Result<()> {
    let (burn_bytes, source_desc) = resolve_full_burn_address(args)?;
    println!("Using FullBurnAddress from {}", source_desc);
    println!("FullBurnAddress    : 0x{}", hex::encode(&burn_bytes));

    let burn_payload =
        FullBurnAddress::from_bytes(&burn_bytes).context("failed to decode FullBurnAddress")?;
    let burn_address = burn_payload
        .burn_address()
        .context("failed to derive burn address from payload")?;
    let recipient_address = Address::from_word(burn_payload.gr.address);
    let chain_id = burn_payload.gr.chain_id;

    println!("Recipient address  : {}", recipient_address);
    println!("Recipient chain ID : {}", chain_id);
    println!("Burn address       : {}", burn_address);

    let hub_entry = hub.ok_or_else(|| anyhow!("hub entry is required to redeem transfers"))?;
    let hub_contract = build_hub(hub_entry)?;
    let token_entry = find_token_by_chain(tokens, chain_id)?;
    let verifier = build_verifier(token_entry)?;
    let token_clients = tokens.iter().map(build_erc20).collect::<Result<Vec<_>>>()?;
    let indexer = build_indexer_client(common_args, "receive transfer command")?;

    let aggregation_tree_state =
        fetch_aggregation_tree_state(common_args.event_block_span, &verifier, &hub_contract)
            .await
            .context("failed to fetch aggregation tree state")?;

    let events_map = fetch_transfer_events(
        &indexer,
        Some(common_args.indexer_fetch_limit),
        tokens,
        &token_clients,
        &[burn_address],
    )
    .await
    .context("failed to fetch transfer events for burn address")?;

    let separated_events = separate_events_by_eligibility(&aggregation_tree_state, &events_map)?;
    let events_for_chain = match separated_events.get(&chain_id) {
        Some(events) => events,
        None => {
            println!(
                "No transfer events found for burn address {} on chain {}.",
                burn_address, chain_id
            );
            return Ok(());
        }
    };

    print_events(chain_id, events_for_chain);

    if events_for_chain.eligible.is_empty() {
        println!(
            "No eligible transfers available for burn address {}.",
            burn_address
        );
        return Ok(());
    }

    let artifacts_dir = common_args.nova_artifacts_dir.as_deref().ok_or_else(|| {
        anyhow!("Nova artifacts directory must be specified for receive transfer command")
    })?;
    let burn_address_to_secret = HashMap::from([(burn_address, burn_payload.secret)]);

    let no_new_transfers_message = format!(
        "Burn address {} has no new eligible transfers to claim.",
        burn_address
    );
    match redeem_transfers(
        common_args,
        &verifier,
        &indexer,
        &aggregation_tree_state,
        &separated_events,
        &burn_address_to_secret,
        burn_payload.gr,
        tokens,
        private_key,
        artifacts_dir,
    )
    .await?
    {
        RedeemResult::AlreadyClaimed => println!("{}", no_new_transfers_message),
        RedeemResult::NoProofs => println!("No claimable Merkle proofs were generated."),
        RedeemResult::Submitted => {}
    }

    Ok(())
}

fn resolve_full_burn_address(args: &ReceiveTransferArgs) -> Result<(Vec<u8>, String)> {
    if let Some(hex) = &args.full_burn_address {
        let bytes = decode_full_burn_address(hex)?;
        return Ok((bytes, "command line --full-burn-address".to_string()));
    }

    let announcement_id = args.announcement_id.ok_or_else(|| {
        anyhow!("announcement_id must be provided if full_burn_address is omitted")
    })?;
    let path = args
        .scan_results_path
        .as_ref()
        .ok_or_else(|| {
            anyhow!(
                "--scan-results-path (or SCAN_RECEIVE_OUTPUT env) is required when using --announcement-id"
            )
        })?;
    let transfer = load_scanned_transfer(path, announcement_id)?;
    let bytes = decode_full_burn_address(&transfer.full_burn_address_hex)?;
    let desc = format!(
        "announcement id {} from {}",
        announcement_id,
        path.display()
    );
    Ok((bytes, desc))
}

fn decode_full_burn_address(input: &str) -> Result<Vec<u8>> {
    let trimmed = input.trim().trim_start_matches("0x");
    let bytes = hex::decode(trimmed)
        .with_context(|| format!("failed to decode FullBurnAddress hex: {}", input))?;
    Ok(bytes)
}

fn load_scanned_transfer(path: &Path, id: u64) -> Result<ScannedTransfer> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open scanned transfers file {}", path.display()))?;
    let entries: Vec<ScannedTransfer> =
        serde_json::from_reader(file).context("failed to parse scanned transfers JSON")?;
    entries
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| anyhow!("announcement id {} not found in {}", id, path.display()))
}
