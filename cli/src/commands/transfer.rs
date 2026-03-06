use std::time::SystemTime;

use alloy::primitives::{B256, U256};
use anyhow::{Context, Result, anyhow, bail};
use client_common::{
    contracts::{
        gelato_relay::{self, RelayTransferParams},
        utils::get_address_from_private_key,
    },
    tokens::TokenEntry,
};

use crate::{
    TransferArgs,
    commands::shared::{build_erc20, build_liquidity_manager, find_token_by_chain, format_tx_hash},
};

pub async fn run(args: &TransferArgs, tokens: &[TokenEntry], private_key: B256) -> Result<()> {
    let entry = find_token_by_chain(tokens, args.chain_id)?;
    let sender = get_address_from_private_key(private_key);

    if args.relay.relay {
        return transfer_via_relay(entry, sender, args, private_key).await;
    }

    let erc20 = build_erc20(entry)?;

    println!("Sender address     : {}", sender);
    println!("Token label        : {}", entry.label);
    println!("Token address      : {}", entry.token_address);
    println!("Recipient address  : {}", args.to);
    println!("Amount (raw)       : {}", args.amount);

    let pending = erc20
        .transfer(private_key, args.to, args.amount)
        .await
        .with_context(|| format!("failed to submit transfer for {}", entry.label))?;

    let tx_hash = format_tx_hash(pending.tx_hash().as_slice());
    println!("Submitted transfer  : {}", tx_hash);

    Ok(())
}

async fn transfer_via_relay(
    entry: &TokenEntry,
    caller: alloy::primitives::Address,
    args: &TransferArgs,
    private_key: B256,
) -> Result<()> {
    let relay_address = entry.gelato_relay_address.ok_or_else(|| {
        anyhow!(
            "token '{}' is missing gelato_relay_address — relay mode not available for this chain",
            entry.label,
        )
    })?;

    let liquidity_manager = build_liquidity_manager(entry)?;
    let fee_token = liquidity_manager
        .underlying_token()
        .await
        .context("failed to fetch underlying token address")?;

    println!("Sender address     : {}", caller);
    println!("Token label        : {}", entry.label);
    println!("Recipient address  : {}", args.to);
    println!("Amount (raw)       : {}", args.amount);
    println!("Relay address      : {}", relay_address);

    // Estimate relayer fee
    println!("Estimating Gelato relay fee...");
    let fee_estimate =
        gelato_relay::estimate_relayer_fee(entry.chain_id, fee_token, None, &liquidity_manager)
            .await
            .context("failed to estimate relayer fee")?;

    let relayer_fee = fee_estimate.relayer_fee;
    if let Some(cap) = args.relay.max_relay_fee
        && cap < relayer_fee
    {
        bail!(
            "estimated relayer fee {} exceeds --max-relay-fee cap {}",
            relayer_fee,
            cap
        );
    }

    let total_amount = args.amount + relayer_fee;
    println!("  Gelato gas fee : {}", fee_estimate.gelato_fee);
    println!("  Unwrap fee     : {}", fee_estimate.unwrap_fee);
    println!("  Relayer fee    : {}", relayer_fee);
    println!("  Total permit   : {}", total_amount);

    // Sign ERC-2612 permit for total_amount (transfer + relayerFee)
    let provider = entry.provider()?;
    let permit_nonce =
        gelato_relay::fetch_permit_nonce(provider.clone(), entry.token_address, caller)
            .await
            .context("failed to fetch permit nonce")?;

    let deadline = U256::from(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .context("system time error")?
            .as_secs()
            + 3600,
    );

    let (v, r, s) = gelato_relay::sign_permit(
        private_key,
        provider.clone(),
        entry.token_address,
        relay_address,
        total_amount,
        permit_nonce,
        deadline,
    )
    .await
    .context("failed to sign ERC-2612 permit")?;
    let mut permit_sig = Vec::with_capacity(65);
    permit_sig.extend_from_slice(r.as_slice());
    permit_sig.extend_from_slice(s.as_slice());
    permit_sig.push(v);

    // Sign relay EIP-712 authorization
    let relay_domain = gelato_relay::fetch_relay_domain_separator(provider.clone(), relay_address)
        .await
        .context("failed to fetch GelatoRelay EIP-712 domain")?;
    let relay_nonce = gelato_relay::fetch_relay_nonce(provider, relay_address, caller)
        .await
        .context("failed to fetch relay nonce")?;
    let relay_sig = gelato_relay::sign_relay_transfer(
        private_key,
        relay_domain,
        caller,
        args.to,
        total_amount,
        relayer_fee,
        fee_estimate.gelato_fee,
        relay_nonce,
    )
    .await
    .context("failed to sign relay transfer authorization")?;

    // Encode calldata
    let params = RelayTransferParams {
        owner: caller,
        to: args.to,
        amount: total_amount,
        relayer_fee,
        max_gelato_fee: fee_estimate.gelato_fee,
        deadline,
        permit_sig,
        relay_sig,
    };
    let calldata = gelato_relay::encode_relay_transfer(&params);

    // Submit to Gelato
    println!("Submitting relay transfer to Gelato...");
    let task_id = gelato_relay::submit_relay_task(
        entry.chain_id,
        relay_address,
        &calldata,
        fee_token,
        args.relay.gelato_api_key.as_deref(),
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
        gelato_relay::RelayTaskState::ExecSuccess => {
            if let Some(tx_hash) = &result.transaction_hash {
                println!("Relay succeeded    : {}", tx_hash);
            } else {
                println!("Relay succeeded (no tx hash available)");
            }
        }
        gelato_relay::RelayTaskState::ExecReverted => {
            let msg = result
                .last_check_message
                .as_deref()
                .unwrap_or("unknown reason");
            bail!("Gelato relay task reverted: {}", msg);
        }
        gelato_relay::RelayTaskState::Cancelled => {
            let msg = result
                .last_check_message
                .as_deref()
                .unwrap_or("unknown reason");
            bail!("Gelato relay task cancelled: {}", msg);
        }
        _ => {
            let msg = result.last_check_message.as_deref().unwrap_or("timed out");
            println!(
                "Relay task still pending after polling: {} — check Gelato status for task {}",
                msg, task_id
            );
        }
    }

    Ok(())
}
