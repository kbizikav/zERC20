use anyhow::{Context, Result, anyhow, bail};
use client_common::layerzero::{ScanMessage, ScanMessagesResponse};
use hex;
use reqwest::Client;

use crate::commands::{
    layerzero_common::{
        ComposeTxSummary, destination_block, destination_status, destination_tx,
        lz_compose_failed_txs, lz_compose_txs, summarize_stage,
    },
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
        if let Some(compose) = destination.get("lzCompose") {
            let compose_txs = lz_compose_txs(compose);
            if !compose_txs.is_empty() {
                println!("Destination lzCompose txs:");
                for tx in &compose_txs {
                    println!("  - {}", tx.summary);
                    print_compose_detail(&client, base, &common.lz_scan_api_key, tx).await?;
                }
            }
            let compose_failed = lz_compose_failed_txs(compose);
            if !compose_failed.is_empty() {
                println!("Destination lzCompose failed:");
                for tx in compose_failed {
                    println!("  - {}", tx);
                }
            }
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

async fn print_compose_detail(
    client: &Client,
    base: &str,
    api_key: &Option<String>,
    tx: &ComposeTxSummary,
) -> Result<()> {
    let url = format!("{}/messages/tx/{}", base, tx.hash);
    let mut request = client.get(&url);
    if let Some(api_key) = api_key {
        request = request.header("x-api-key", api_key);
    }

    let response = request.send().await;
    let response = match response {
        Ok(resp) => resp,
        Err(err) => {
            println!("    (fetch failed: {})", err);
            return Ok(());
        }
    };

    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    if !status.is_success() {
        println!("    (fetch returned {} for compose tx)", status);
        return Ok(());
    }

    let parsed: ScanMessagesResponse<ScanMessage> = response.json().await.unwrap_or_else(|err| {
        println!("    (failed to parse compose tx response: {})", err);
        ScanMessagesResponse {
            data: vec![],
            next_token: None,
        }
    });

    if let Some(message) = parsed.data.get(0) {
        let (_source_status, source_tx, source_block) = summarize_stage(message.source.as_ref());
        println!(
            "    status: {} ({})",
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
        println!("    source tx: {}", source_tx);
        if let Some(block) = source_block {
            println!("    source block: {}", block);
        }
        if let Some(destination) = &message.destination {
            let dest_status = destination_status(destination).unwrap_or_else(|| "-".to_string());
            let dest_tx = destination_tx(destination).unwrap_or_else(|| "-".to_string());
            let dest_block = destination_block(destination);
            println!("    dest status: {}", dest_status);
            println!("    dest tx: {}", dest_tx);
            if let Some(block) = dest_block {
                println!("    dest block: {}", block);
            }
        }
    }

    Ok(())
}
