use alloy::{
    primitives::{Address, B256, U256},
    sol_types::{SolCall, SolValue},
};
use anyhow::{Context, Result, anyhow};
use client_common::{
    contracts::utils::get_address_from_private_key,
    contracts::{adaptor::Adaptor, z_erc20::zERC20},
    layerzero::{
        HttpLayerZeroClient, LayerZeroClient, LzCompose, ScanMessage, WalletMessagesParams,
    },
    tokens::{load_tokens_from_path, TokenEntry},
};
use hex;
use reqwest::{Client as HttpClient, Url};
use serde::Deserialize;

use crate::commands::layerzero_common::{
    destination_block, destination_status, destination_tx, summarize_stage,
};
use crate::{CommonArgs, WalletMessagesArgs};

pub async fn run(common: &CommonArgs, args: &WalletMessagesArgs, private_key: B256) -> Result<()> {
    let base = Url::parse(&common.lz_scan_api_url).with_context(|| {
        format!(
            "invalid LZ_SCAN_API_URL '{}' for wallet messages",
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
    let http = HttpClient::new();

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
        print_message(idx, message, &tokens, &http, &client).await?;
    }

    Ok(())
}

async fn print_message(
    index: usize,
    message: &ScanMessage,
    tokens: &[TokenEntry],
    http: &HttpClient,
    client: &impl LayerZeroClient,
) -> Result<()> {
    println!("Message {}:", index + 1);
    println!(
        "  GUID               : {}",
        message.guid.as_deref().unwrap_or("-")
    );
    println!(
        "  Status             : {} ({})",
        message
            .status
            .as_ref()
            .and_then(|s| s.name.as_deref())
            .unwrap_or("-"),
        message
            .status
            .as_ref()
            .and_then(|s| s.message.as_deref())
            .unwrap_or("-")
    );

    if let Some(pathway) = &message.pathway {
        println!(
            "  Pathway            : {} -> {} (nonce {})",
            pathway
                .sender
                .as_ref()
                .and_then(|p| p.chain.as_deref())
                .unwrap_or("-"),
            pathway
                .receiver
                .as_ref()
                .and_then(|p| p.chain.as_deref())
                .unwrap_or("-"),
            pathway.nonce.unwrap_or_default()
        );
        println!(
            "  Endpoint IDs       : {} -> {}",
            pathway.src_eid.unwrap_or_default(),
            pathway.dst_eid.unwrap_or_default()
        );
        println!(
            "  Sender -> Receiver : {} -> {}",
            pathway
                .sender
                .as_ref()
                .and_then(|p| p.address.as_deref())
                .unwrap_or("-"),
            pathway
                .receiver
                .as_ref()
                .and_then(|p| p.address.as_deref())
                .unwrap_or("-")
        );
        if let Some(pathway_id) = pathway.id.as_deref() {
            println!("  Pathway ID         : {}", pathway_id);
        }
    } else {
        println!("  Pathway            : -");
    }

    let (source_status, source_tx, source_block) = summarize_stage(message.source.as_ref());
    println!("  Source status      : {}", source_status);
    println!("  Source tx          : {}", source_tx);
    if let Some(block) = source_block {
        println!("  Source block       : {}", block);
    }
    let payload = message
        .source
        .as_ref()
        .and_then(|stage| stage.tx.as_ref())
        .and_then(|tx| tx.payload.as_deref());
    let source_tx_hash = message
        .source
        .as_ref()
        .and_then(|stage| stage.tx.as_ref())
        .and_then(|tx| tx.tx_hash.as_deref());
    match source_tx_hash {
        Some(hash) => match fetch_and_decode_send(hash, message, tokens, http).await {
            Ok(Some(summary)) => {
                println!(
                    "  Source payload     : zERC20.send dstEid={} to={} amount={} minAmount={} extraOptions={}B composeMsg={}B oftCmd={}B",
                    summary.dst_eid,
                    summary.to,
                    summary.amount,
                    summary.min_amount,
                    summary.extra_options_len,
                    summary.compose_msg_len,
                    summary.oft_cmd_len
                );
                if let Some(compose) = summary.compose {
                    println!(
                        "    compose BridgeRequest dstEid={} to={} refund={} minOut={} extraOptions={}B composeMsg={}B oftCmd={}B",
                        compose.dst_eid,
                        compose.to,
                        compose.refund_address,
                        compose.min_amount_out,
                        compose.extra_options_len,
                        compose.compose_msg_len,
                        compose.oft_cmd_len
                    );
                }
            }
            Ok(None) => println!("  Source payload     : (tx input empty or not found)"),
            Err(err) => println!(
                "  Source payload     : (failed to decode zERC20.send from tx: {})",
                err
            ),
        },
        None => match payload {
            Some(raw) => match decode_send_payload(raw) {
                Ok(summary) => {
                    println!(
                        "  Source payload     : zERC20.send dstEid={} to={} amount={} minAmount={} extraOptions={}B composeMsg={}B oftCmd={}B",
                        summary.dst_eid,
                        summary.to,
                        summary.amount,
                        summary.min_amount,
                        summary.extra_options_len,
                        summary.compose_msg_len,
                        summary.oft_cmd_len
                    );
                    if let Some(compose) = summary.compose {
                        println!(
                            "    compose BridgeRequest dstEid={} to={} refund={} minOut={} extraOptions={}B composeMsg={}B oftCmd={}B",
                            compose.dst_eid,
                            compose.to,
                            compose.refund_address,
                            compose.min_amount_out,
                            compose.extra_options_len,
                            compose.compose_msg_len,
                            compose.oft_cmd_len
                        );
                    }
                }
                Err(err) => println!(
                    "  Source payload     : (failed to decode zERC20.send; len {}B; err: {})",
                    hex_len(raw),
                    err
                ),
            },
            None => println!("  Source payload     : -"),
        },
    }

    if let Some(destination) = &message.destination {
        let dest_status = destination_status(destination).unwrap_or_else(|| "-".to_string());
        let dest_tx = destination_tx(destination).unwrap_or_else(|| "-".to_string());
        let dest_block = destination_block(destination);
        println!("  Destination status : {}", dest_status);
        println!("  Destination tx     : {}", dest_tx);
        if let Some(block) = dest_block {
            println!("  Destination block  : {}", block);
        }
        if let Some(compose) = destination.lz_compose.as_ref() {
            let followups = fetch_compose_followups(client, compose).await;
            if !followups.is_empty() {
                println!("  lzCompose follow-ups:");
                for followup in followups {
                    println!(
                        "    - {} -> {} | source tx {} | dest tx {} | success {}",
                        followup.src_chain,
                        followup.dst_chain,
                        followup.source_tx,
                        followup.dest_tx,
                        followup.success
                    );
                }
            }
        }
    } else {
        println!("  Destination status : -");
        println!("  Destination tx     : -");
    }

    println!(
        "  DVN status         : {}",
        message
            .verification
            .as_ref()
            .and_then(|v| v.dvn.as_ref())
            .and_then(|d| d.status.as_deref())
            .unwrap_or("-")
    );
    println!(
        "  Sealer status      : {}",
        message
            .sealer
            .as_ref()
            .and_then(|s| s.status.as_deref())
            .unwrap_or("-")
    );

    Ok(())
}

#[derive(Debug)]
struct SendPayloadSummary {
    dst_eid: u32,
    to: Address,
    amount: U256,
    min_amount: U256,
    extra_options_len: usize,
    compose_msg_len: usize,
    oft_cmd_len: usize,
    compose: Option<BridgeRequestSummary>,
}

#[derive(Debug)]
struct BridgeRequestSummary {
    dst_eid: u32,
    to: Address,
    refund_address: Address,
    min_amount_out: U256,
    extra_options_len: usize,
    compose_msg_len: usize,
    oft_cmd_len: usize,
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
        extra_options_len: param.extraOptions.len(),
        compose_msg_len: param.composeMsg.len(),
        oft_cmd_len: param.oftCmd.len(),
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
        extra_options_len: request.extraOptions.len(),
        compose_msg_len: request.composeMsg.len(),
        oft_cmd_len: request.oftCmd.len(),
    }))
}

fn hex_len(hex_str: &str) -> usize {
    hex_str.trim_start_matches("0x").len() / 2
}

fn decode_send_call(bytes: &[u8]) -> Result<zERC20::sendCall> {
    let mut last_err = match zERC20::sendCall::abi_decode(bytes) {
        Ok(call) => return Ok(call),
        Err(err) => err,
    };

    if let Ok(call) = zERC20::sendCall::abi_decode_raw(bytes) {
        return Ok(call);
    }

    if bytes.len() > 4 {
        match zERC20::sendCall::abi_decode(&bytes[4..]) {
            Ok(call) => return Ok(call),
            Err(err) => last_err = err,
        }
    }

    Err(anyhow!(last_err))
}

fn decode_bridge_request_inner(bytes: &[u8]) -> Result<Adaptor::BridgeRequest> {
    let mut last_err = match Adaptor::BridgeRequest::abi_decode(bytes) {
        Ok(request) => return Ok(request),
        Err(err) => err,
    };

    if bytes.len() > 4 {
        match Adaptor::BridgeRequest::abi_decode(&bytes[4..]) {
            Ok(request) => return Ok(request),
            Err(err) => last_err = err,
        }
    }

    Err(anyhow!(last_err))
}

#[derive(Debug, Deserialize)]
struct EthTx {
    #[serde(default)]
    input: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RpcResponse {
    result: Option<EthTx>,
    #[serde(default)]
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    message: Option<String>,
}

async fn fetch_and_decode_send(
    tx_hash: &str,
    message: &ScanMessage,
    tokens: &[TokenEntry],
    http: &HttpClient,
) -> Result<Option<SendPayloadSummary>> {
    let entry = find_token_for_message(message, tokens)
        .ok_or_else(|| anyhow!("no token entry matched source pathway to fetch tx"))?;
    let calldata = fetch_tx_input(http, entry, tx_hash).await?;
    let Some(input) = calldata else {
        return Ok(None);
    };
    let summary = decode_send_payload(&input)?;
    Ok(Some(summary))
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

async fn fetch_tx_input(
    http: &HttpClient,
    entry: &TokenEntry,
    tx_hash: &str,
) -> Result<Option<String>> {
    for url in &entry.rpc_urls {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getTransactionByHash",
            "params": [tx_hash],
        });
        match http.post(url).json(&body).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    continue;
                }
                let parsed: RpcResponse = resp.json().await.unwrap_or(RpcResponse {
                    result: None,
                    error: None,
                });
                if let Some(err) = parsed.error {
                    return Err(anyhow!(
                        "rpc error from {}: {}",
                        url,
                        err.message.unwrap_or_else(|| "unknown error".into())
                    ));
                }
                if let Some(result) = parsed.result {
                    return Ok(result.input);
                }
            }
            Err(_) => continue,
        }
    }
    Ok(None)
}

#[derive(Debug)]
struct ComposeFollowUp {
    src_chain: String,
    dst_chain: String,
    source_tx: String,
    dest_tx: String,
    success: bool,
}

async fn fetch_compose_followups(
    client: &impl LayerZeroClient,
    compose: &LzCompose,
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
            .and_then(|p| p.chain.as_deref())
            .unwrap_or("-")
            .to_string();
        let dst_chain = message
            .pathway
            .as_ref()
            .and_then(|p| p.receiver.as_ref())
            .and_then(|p| p.chain.as_deref())
            .unwrap_or("-")
            .to_string();

        let (_source_status, source_tx, _) = summarize_stage(message.source.as_ref());
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

        out.push(ComposeFollowUp {
            src_chain,
            dst_chain,
            source_tx,
            dest_tx,
            success,
        });
    }
    out
}
