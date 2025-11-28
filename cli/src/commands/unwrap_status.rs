use anyhow::{Context, Result, anyhow, bail};
use client_common::layerzero::{ScanMessage, ScanMessagesResponse};
use hex;
use reqwest::Client;

use crate::commands::{
    layerzero_common::{destination_block, destination_status, destination_tx, summarize_stage},
    shared::parse_b256,
};
use crate::{CommonArgs, UnwrapStatusArgs};

pub async fn run(common: &CommonArgs, args: &UnwrapStatusArgs) -> Result<()> {
    let tx_hash = normalize_tx_hash(&args.tx_hash)?;
    let client = Client::builder()
        .user_agent("curl/8.0 (zerc20-cli layerzero client)")
        .build()
        .context("failed to build LayerZero Scan HTTP client")?;
    let base = common.lz_scan_api_url.trim_end_matches('/');
    let url = format!("{}/messages/tx/{}", base, tx_hash);

    let mut request = client.get(&url);
    if let Some(api_key) = &common.lz_scan_api_key {
        request = request.header("x-api-key", api_key);
    }

    let response = request
        .send()
        .await
        .with_context(|| format!("failed to query LayerZero Scan at {}", url))?;

    let status_code = response.status();
    let body = response
        .text()
        .await
        .context("failed to read LayerZero Scan response body")?;

    if !status_code.is_success() {
        bail!(
            "LayerZero Scan responded with {}: {}",
            status_code.as_u16(),
            body
        );
    }

    let parsed: ScanMessagesResponse<ScanMessage> = serde_json::from_str(&body)
        .with_context(|| format!("failed to parse LayerZero Scan response: {}", body))?;

    let message = parsed
        .data
        .get(0)
        .with_context(|| format!("no message found for {}", tx_hash))?;

    println!("LayerZero Scan URL  : {}", url);
    println!(
        "Message status      : {} ({})",
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
    println!(
        "GUID                : {}",
        message.guid.as_deref().unwrap_or("-")
    );

    if let Some(pathway) = &message.pathway {
        println!(
            "Pathway             : {} -> {} (nonce {})",
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
            pathway.nonce.unwrap_or_default(),
        );
        println!(
            "Endpoint IDs        : {} -> {}",
            pathway.src_eid.unwrap_or_default(),
            pathway.dst_eid.unwrap_or_default()
        );
        println!(
            "Sender -> Receiver  : {} -> {}",
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
            println!("Pathway ID          : {}", pathway_id);
        }
    } else {
        println!("Pathway             : -");
    }

    let (source_status, source_tx, source_block) = summarize_stage(message.source.as_ref());
    println!("Source status       : {}", source_status);
    println!("Source tx           : {}", source_tx);
    if let Some(block) = source_block {
        println!("Source block        : {}", block);
    }

    if let Some(destination) = &message.destination {
        let dest_status = destination_status(destination).unwrap_or_else(|| "-".to_string());
        let dest_tx = destination_tx(destination).unwrap_or_else(|| "-".to_string());
        let dest_block = destination_block(destination);
        println!("Destination status  : {}", dest_status);
        println!("Destination tx      : {}", dest_tx);
        if let Some(block) = dest_block {
            println!("Destination block   : {}", block);
        }
    } else {
        println!("Destination status  : -");
        println!("Destination tx      : -");
    }

    println!(
        "DVN status          : {}",
        message
            .verification
            .as_ref()
            .and_then(|v| v.dvn.as_ref())
            .and_then(|d| d.status.as_deref())
            .unwrap_or("-")
    );
    println!(
        "Sealer status       : {}",
        message
            .sealer
            .as_ref()
            .and_then(|s| s.status.as_deref())
            .unwrap_or("-")
    );

    Ok(())
}

fn normalize_tx_hash(value: &str) -> Result<String> {
    let parsed = parse_b256(value).map_err(|err| anyhow!(err))?;
    Ok(format!("0x{}", hex::encode(parsed.as_slice())))
}
