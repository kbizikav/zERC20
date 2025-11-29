use alloy::{
    primitives::{Address, B256, U256},
    sol_types::{SolCall, SolValue},
};
use anyhow::{Context, Result, anyhow};
use client_common::{
    contracts::utils::{NormalProvider, fetch_tx_input, get_address_from_private_key},
    contracts::{adaptor::Adaptor, z_erc20::zERC20},
    layerzero::{
        Destination, Endpoint, HttpLayerZeroClient, LayerZeroClient, LzCompose, ScanMessage, Stage,
        WalletMessagesParams,
    },
    tokens::{TokenEntry, load_tokens_from_path},
};
use reqwest::Url;

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
                        let amount = match (
                            followup.amount_sent_ld,
                            followup.amount_received_ld,
                            followup.amount,
                        ) {
                            (Some(sent), Some(received), _) => format!("{sent}->{received}"),
                            (None, None, Some(raw)) => raw.to_string(),
                            _ => "-".to_string(),
                        };
                        println!(
                            "    - {} -> {} | src tx {} | dst tx {} | amount {} | success {}",
                            followup.src_chain,
                            followup.dst_chain,
                            followup.source_tx,
                            followup.dest_tx,
                            amount,
                            followup.success
                        );
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
    dst_eid: u32,
    to: Address,
    amount: U256,
    min_amount: U256,
    amount_sent_ld: Option<U256>,
    amount_received_ld: Option<U256>,
    compose: Option<BridgeRequestSummary>,
}

#[derive(Debug)]
struct BridgeRequestSummary {
    dst_eid: u32,
    to: Address,
    refund_address: Address,
    min_amount_out: U256,
}

fn print_send_summary(summary: &SendPayloadSummary) {
    println!(
        "  Send      : dstEid={} to={} amount={} minAmount={}",
        summary.dst_eid, summary.to, summary.amount, summary.min_amount
    );
    if let (Some(amount_in), Some(amount_out)) =
        (summary.amount_sent_ld, summary.amount_received_ld)
    {
        println!("    Flow     : in={} out={}", amount_in, amount_out);
    }
    if let Some(compose) = &summary.compose {
        println!(
            "    Compose  : dstEid={} to={} refund={} minOut={}",
            compose.dst_eid, compose.to, compose.refund_address, compose.min_amount_out
        );
    }
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
    let compose = decode_bridge_request(&param.composeMsg)?;

    Ok(SendPayloadSummary {
        dst_eid: param.dstEid,
        to: Address::from_word(param.to),
        amount: param.amountLD,
        min_amount: param.minAmountLD,
        amount_sent_ld: None,
        amount_received_ld: None,
        compose,
    })
}

fn decode_bridge_request(bytes: &[u8]) -> Result<Option<BridgeRequestSummary>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let request = decode_bridge_request_inner(bytes)
        .map_err(|err| anyhow!("decode compose BridgeRequest: {err}"))?;
    Ok(Some(BridgeRequestSummary {
        dst_eid: request.dstEid,
        to: request.to,
        refund_address: request.refundAddress,
        min_amount_out: request.minAmountOut,
    }))
}

fn apply_decimal_conversion(summary: &mut SendPayloadSummary, rate: U256) {
    if rate == U256::ZERO {
        return;
    }

    let adjusted = summary.amount - (summary.amount % rate);
    summary.amount_sent_ld = Some(adjusted);
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

fn decode_send_call(bytes: &[u8]) -> Result<zERC20::sendCall> {
    if let Ok(call) = zERC20::sendCall::abi_decode(bytes) {
        return Ok(call);
    }

    if let Ok(call) = zERC20::sendCall::abi_decode_raw(bytes) {
        return Ok(call);
    }

    if bytes.len() > 4 {
        if let Ok(call) = zERC20::sendCall::abi_decode(&bytes[4..]) {
            return Ok(call);
        }
    }

    Err(anyhow!("zERC20::sendCall decode failed"))
}

fn decode_bridge_request_inner(bytes: &[u8]) -> Result<Adaptor::BridgeRequest> {
    if let Ok(request) = Adaptor::BridgeRequest::abi_decode(bytes) {
        return Ok(request);
    }

    if bytes.len() > 4 {
        if let Ok(request) = Adaptor::BridgeRequest::abi_decode(&bytes[4..]) {
            return Ok(request);
        }
    }

    Err(anyhow!("Adaptor::BridgeRequest decode failed"))
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
        if let Some(entry) = token_entry {
            if let Ok(provider) = entry.provider() {
                if let Some(rate) =
                    fetch_decimal_conversion_rate(&provider, entry.token_address).await
                {
                    apply_decimal_conversion(&mut summary, rate);
                }
            }
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
        if let Some(eid) = src_eid {
            if t.eid == Some(eid) {
                return true;
            }
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
    amount: Option<U256>,
    amount_sent_ld: Option<U256>,
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
        let Some(message) = response.data.get(0) else {
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

        let send_summary = summarize_send(message, tokens).await.ok().flatten();

        out.push(ComposeFollowUp {
            src_chain,
            dst_chain,
            source_tx,
            dest_tx,
            success,
            amount: send_summary.as_ref().map(|s| s.amount),
            amount_sent_ld: send_summary.as_ref().and_then(|s| s.amount_sent_ld),
            amount_received_ld: send_summary.as_ref().and_then(|s| s.amount_received_ld),
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
