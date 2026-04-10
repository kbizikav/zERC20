// SPDX-License-Identifier: BUSL-1.1

use alloy::{
    primitives::{Address, B256, U256},
    providers::Provider,
    sol_types::{SolCall, SolEvent},
};
use anyhow::{Context, Result, anyhow};
use client_common::{
    contracts::{
        utils::{NormalProvider, fetch_tx_input, get_address_from_private_key},
        z_erc20::zERC20,
    },
    layerzero::{
        Destination, Endpoint, HttpLayerZeroClient, LayerZeroClient, LzCompose, ScanMessage, Stage,
        WalletMessagesParams,
    },
    tokens::{TokenEntry, load_tokens_from_path},
};
use reqwest::Url;
use std::str::FromStr;

use crate::{CommonArgs, LzStatusArgs};

pub async fn run(common: &CommonArgs, args: &LzStatusArgs, private_key: B256) -> Result<()> {
    let base = Url::parse(&common.lz_scan_api_url).with_context(|| {
        format!(
            "invalid LZ_SCAN_API_URL '{}' for lz-status",
            common.lz_scan_api_url
        )
    })?;
    let client = HttpLayerZeroClient::new(base.clone(), common.lz_scan_api_key.clone())
        .context("failed to construct LayerZero Scan client")?;
    let address = get_address_from_private_key(private_key);

    let tokens = load_tokens_from_path(&common.tokens_file_path)
        .with_context(|| {
            format!(
                "failed to load tokens file {}",
                common.tokens_file_path.display()
            )
        })?
        .tokens;

    let params = WalletMessagesParams {
        limit: Some(args.limit),
        start: args.start.clone(),
        end: args.end.clone(),
        next_token: args.next_token.clone(),
    };

    let response = client
        .wallet_messages(address, &params)
        .await
        .context("failed to fetch LayerZero Scan wallet messages")?;

    let url_prefix = common.lz_scan_api_url.trim_end_matches('/');
    println!(
        "LayerZero Scan URL  : {}/messages/wallet/{:#x}",
        url_prefix, address
    );
    if let Some(next) = &response.next_token {
        println!("Next token          : {}", next);
    }
    println!("Messages returned   : {}", response.data.len());

    if response.data.is_empty() {
        println!("No messages found for wallet");
        return Ok(());
    }

    for (idx, message) in response.data.iter().enumerate() {
        print_message(idx, message, &tokens, &client).await?;
    }

    Ok(())
}

async fn print_message(
    index: usize,
    message: &ScanMessage,
    tokens: &[TokenEntry],
    client: &impl LayerZeroClient,
) -> Result<()> {
    println!("Message {}:", index + 1);
    println!("  GUID       : {}", message.guid.as_deref().unwrap_or("-"));

    match message.pathway.as_ref() {
        Some(pathway) => {
            let src_chain = endpoint_chain(pathway.sender.as_ref());
            let dst_chain = endpoint_chain(pathway.receiver.as_ref());
            let nonce = pathway.nonce.unwrap_or_default();
            println!(
                "  Pathway    : {} -> {} (nonce {})",
                src_chain, dst_chain, nonce
            );
        }
        None => println!("  Pathway    : -"),
    }

    let StageSummary {
        tx_hash: source_tx,
        block: source_block,
        ..
    } = summarize_stage(message.source.as_ref());
    println!("  Source tx  : {}", source_tx);
    if let Some(block) = source_block {
        println!("  Source blk : {}", block);
    }

    print_send_details(message, tokens).await?;

    match &message.destination {
        Some(destination) => {
            let dest_tx = destination_tx(destination).unwrap_or_else(|| "-".to_string());
            println!("  Dest tx    : {}", dest_tx);
            if let Some(compose) = destination.lz_compose.as_ref() {
                let followups = fetch_compose_followups(client, compose, tokens).await;
                if !followups.is_empty() {
                    println!("  Compose tx :");
                    for followup in followups {
                        let output_amount = followup
                            .amount_received_ld
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_string());
                        println!(
                            "    Pathway: {} -> {}",
                            followup.src_chain, followup.dst_chain
                        );
                        println!("      src tx   : {}", followup.source_tx);
                        println!("      dst tx   : {}", followup.dest_tx);
                        println!("      Output amount (underlying): {}", output_amount);
                        println!("      success  : {}", followup.success);
                    }
                }
            }
        }
        None => println!("  Dest tx    : -"),
    }

    Ok(())
}

#[derive(Debug)]
struct StageSummary {
    tx_hash: String,
    block: Option<String>,
}

fn summarize_stage(stage: Option<&Stage>) -> StageSummary {
    let Some(stage) = stage else {
        return StageSummary {
            tx_hash: "-".into(),
            block: None,
        };
    };

    let tx_hash = stage
        .tx
        .as_ref()
        .and_then(|tx| tx.tx_hash.as_deref())
        .unwrap_or("-")
        .to_string();
    let block = stage.tx.as_ref().and_then(|tx| {
        tx.block_number.map(|number| match tx.block_timestamp {
            Some(ts) => format!("{number} (timestamp {ts})"),
            None => number.to_string(),
        })
    });

    StageSummary { tx_hash, block }
}

fn endpoint_chain(endpoint: Option<&Endpoint>) -> &str {
    endpoint.and_then(|p| p.chain.as_deref()).unwrap_or("-")
}

fn source_tx_hash(message: &ScanMessage) -> Option<&str> {
    message
        .source
        .as_ref()
        .and_then(|stage| stage.tx.as_ref())
        .and_then(|tx| tx.tx_hash.as_deref())
}

fn source_payload(message: &ScanMessage) -> Option<&str> {
    message
        .source
        .as_ref()
        .and_then(|stage| stage.tx.as_ref())
        .and_then(|tx| tx.payload.as_deref())
}

#[derive(Debug)]
struct SendPayloadSummary {
    amount: U256,
    amount_sent_ld: Option<U256>,
    amount_received_ld: Option<U256>,
}

fn print_send_summary(summary: &SendPayloadSummary) {
    let input_amount = summary.amount_sent_ld.unwrap_or(summary.amount);

    println!("  Input amount (zERC20)      : {}", input_amount);
}

async fn print_send_details(message: &ScanMessage, tokens: &[TokenEntry]) -> Result<()> {
    let had_hash = source_tx_hash(message).is_some();
    let had_payload = source_payload(message).is_some();
    match summarize_send(message, tokens).await {
        Ok(Some(summary)) => print_send_summary(&summary),
        Ok(None) => {
            if had_hash || had_payload {
                println!("  Send      : (tx input empty or not found)");
            } else {
                println!("  Send      : -");
            }
        }
        Err(err) => println!("  Send      : (decode failed from tx: {})", err),
    }
    Ok(())
}

fn decode_send_payload(payload_hex: &str) -> Result<SendPayloadSummary> {
    let bytes = hex::decode(payload_hex.trim_start_matches("0x"))
        .map_err(|err| anyhow!("hex decode failed: {err}"))?;
    let expected_selector = hex::encode(zERC20::sendCall::SELECTOR);
    let found_selector = bytes
        .get(0..4)
        .map(hex::encode)
        .unwrap_or_else(|| "-".to_string());

    let call = decode_send_call(&bytes).map_err(|err| {
        anyhow!(
            "abi decode failed (found selector 0x{}, expected 0x{}): {}",
            found_selector,
            expected_selector,
            err
        )
    })?;
    let param = call._sendParam;

    Ok(SendPayloadSummary {
        amount: param.amountLD,
        amount_sent_ld: None,
        amount_received_ld: None,
    })
}

fn apply_decimal_conversion(summary: &mut SendPayloadSummary, rate: U256) {
    if rate == U256::ZERO {
        return;
    }

    let adjusted = summary.amount - (summary.amount % rate);
    summary.amount_sent_ld = Some(summary.amount);
    summary.amount_received_ld = Some(adjusted);
}

async fn fetch_decimal_conversion_rate(
    provider: &NormalProvider,
    token_address: Address,
) -> Option<U256> {
    zERC20::new(token_address, provider.clone())
        .decimalConversionRate()
        .call()
        .await
        .ok()
}

async fn fetch_oft_sent_amounts(provider: &NormalProvider, tx_hash: &str) -> Option<U256> {
    let hash = B256::from_str(tx_hash).ok()?;
    let receipt = provider.get_transaction_receipt(hash).await.ok()??;
    for log in receipt.logs() {
        if let Ok(decoded) = zERC20::OFTSent::decode_log(&log.inner) {
            let evt = decoded.data;
            return Some(evt.amountReceivedLD);
        }
    }
    None
}

fn decode_send_call(bytes: &[u8]) -> Result<zERC20::sendCall> {
    if let Ok(call) = zERC20::sendCall::abi_decode(bytes) {
        return Ok(call);
    }

    if let Ok(call) = zERC20::sendCall::abi_decode_raw(bytes) {
        return Ok(call);
    }

    if bytes.len() > 4
        && let Ok(call) = zERC20::sendCall::abi_decode(&bytes[4..])
    {
        return Ok(call);
    }

    Err(anyhow!("zERC20::sendCall decode failed"))
}

async fn fetch_and_decode_send(
    tx_hash: &str,
    entry: &TokenEntry,
) -> Result<Option<SendPayloadSummary>> {
    let provider = entry.provider()?;
    let calldata = fetch_tx_input(&provider, tx_hash)
        .await
        .map_err(|err| anyhow!(err))?;
    let Some(input) = calldata else {
        return Ok(None);
    };
    let mut summary = decode_send_payload(&input)?;
    if let Some(rate) = fetch_decimal_conversion_rate(&provider, entry.token_address).await {
        apply_decimal_conversion(&mut summary, rate);
    }
    Ok(Some(summary))
}

async fn summarize_send(
    message: &ScanMessage,
    tokens: &[TokenEntry],
) -> Result<Option<SendPayloadSummary>> {
    let token_entry = find_token_for_message(message, tokens);
    if let Some(hash) = source_tx_hash(message) {
        let entry = token_entry
            .ok_or_else(|| anyhow!("no token entry matched source pathway to fetch tx"))?;
        return fetch_and_decode_send(hash, entry).await;
    }

    if let Some(raw) = source_payload(message) {
        let mut summary = decode_send_payload(raw)?;
        if let Some(entry) = token_entry
            && let Ok(provider) = entry.provider()
            && let Some(rate) = fetch_decimal_conversion_rate(&provider, entry.token_address).await
        {
            apply_decimal_conversion(&mut summary, rate);
        }
        return Ok(Some(summary));
    }

    Ok(None)
}

fn find_token_for_message<'a>(
    message: &ScanMessage,
    tokens: &'a [TokenEntry],
) -> Option<&'a TokenEntry> {
    let src_eid: Option<u32> = message
        .pathway
        .as_ref()
        .and_then(|p| p.src_eid)
        .and_then(|v| v.try_into().ok());
    let src_chain = message
        .pathway
        .as_ref()
        .and_then(|p| p.sender.as_ref())
        .and_then(|s| s.chain.as_deref())
        .map(str::to_lowercase);

    tokens.iter().find(|t| {
        if let Some(eid) = src_eid
            && t.eid == Some(eid)
        {
            return true;
        }
        if let Some(chain) = src_chain.as_deref() {
            return t.label.to_lowercase() == chain;
        }
        false
    })
}

#[derive(Debug)]
struct ComposeFollowUp {
    src_chain: String,
    dst_chain: String,
    source_tx: String,
    dest_tx: String,
    success: bool,
    amount_received_ld: Option<U256>,
}

async fn fetch_compose_followups(
    client: &impl LayerZeroClient,
    compose: &LzCompose,
    tokens: &[TokenEntry],
) -> Vec<ComposeFollowUp> {
    let mut out = Vec::new();
    for tx in &compose.txs {
        let Some(hash) = tx.tx_hash.as_deref() else {
            continue;
        };

        let Ok(Some(response)) = client.tx_messages(hash).await else {
            continue;
        };
        let Some(message) = response.data.first() else {
            continue;
        };

        let src_chain = message
            .pathway
            .as_ref()
            .and_then(|p| p.sender.as_ref())
            .map(|endpoint| endpoint_chain(Some(endpoint)))
            .unwrap_or("-")
            .to_string();
        let dst_chain = message
            .pathway
            .as_ref()
            .and_then(|p| p.receiver.as_ref())
            .map(|endpoint| endpoint_chain(Some(endpoint)))
            .unwrap_or("-")
            .to_string();

        let source_tx = summarize_stage(message.source.as_ref()).tx_hash;
        let dest_tx = message
            .destination
            .as_ref()
            .and_then(destination_tx)
            .unwrap_or_else(|| "-".to_string());

        let success = message
            .status
            .as_ref()
            .and_then(|s| s.name.as_deref())
            .map(|name| name.eq_ignore_ascii_case("delivered"))
            .unwrap_or(false)
            || message
                .destination
                .as_ref()
                .and_then(|d| d.status.as_deref())
                .map(|name| name.eq_ignore_ascii_case("delivered"))
                .unwrap_or(false);

        let token_entry = find_token_for_message(message, tokens);
        let send_summary = summarize_send(message, tokens).await.ok().flatten();

        let mut amount_received_ld = send_summary.as_ref().and_then(|s| s.amount_received_ld);

        if let Some(entry) = token_entry
            && let Ok(provider) = entry.provider()
            && let Some(received) = fetch_oft_sent_amounts(&provider, hash).await
        {
            amount_received_ld = Some(received);
        }

        out.push(ComposeFollowUp {
            src_chain,
            dst_chain,
            source_tx,
            dest_tx,
            success,
            amount_received_ld,
        });
    }
    out
}

fn destination_tx(destination: &Destination) -> Option<String> {
    destination
        .tx
        .as_ref()
        .and_then(|tx| tx.tx_hash.clone())
        .or_else(|| {
            destination
                .lz_compose
                .as_ref()
                .and_then(|lz| lz.txs.first())
                .and_then(|tx| tx.tx_hash.clone())
        })
        .or_else(|| {
            destination
                .native_drop
                .as_ref()
                .and_then(|drop| drop.tx.as_ref())
                .and_then(|tx| tx.tx_hash.clone())
        })
}
