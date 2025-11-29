use alloy::{
    primitives::{Address, B256, U256},
    sol_types::{SolCall as _, SolValue as _},
};
use anyhow::{Context, Result};
use client_common::{
    contracts::utils::get_address_from_private_key,
    contracts::{adaptor::Adaptor, z_erc20::zERC20},
    layerzero::{
        HttpLayerZeroClient, LayerZeroClient, LzCompose, ScanMessage, WalletMessagesParams,
    },
};
use hex;
use reqwest::Url;

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
        print_message(idx, message, &client).await?;
    }

    Ok(())
}

async fn print_message(
    index: usize,
    message: &ScanMessage,
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
    if let Some(summary) = payload.and_then(decode_send_payload) {
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
    } else {
        println!("  Source payload     : -");
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

fn decode_send_payload(payload_hex: &str) -> Option<SendPayloadSummary> {
    let bytes = hex::decode(payload_hex.trim_start_matches("0x")).ok()?;
    let call = zERC20::sendCall::abi_decode(&bytes).ok()?;
    let param = call._sendParam;
    let compose = decode_bridge_request(&param.composeMsg);

    Some(SendPayloadSummary {
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

fn decode_bridge_request(bytes: &[u8]) -> Option<BridgeRequestSummary> {
    if bytes.is_empty() {
        return None;
    }
    let request = Adaptor::BridgeRequest::abi_decode(bytes).ok()?;
    Some(BridgeRequestSummary {
        dst_eid: request.dstEid,
        to: request.to,
        refund_address: request.refundAddress,
        min_amount_out: request.minAmountOut,
        extra_options_len: request.extraOptions.len(),
        compose_msg_len: request.composeMsg.len(),
        oft_cmd_len: request.oftCmd.len(),
    })
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
