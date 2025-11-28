use alloy::primitives::B256;
use anyhow::{Context, Result};
use client_common::{
    contracts::utils::get_address_from_private_key,
    layerzero::{HttpLayerZeroClient, LayerZeroClient, ScanMessage, WalletMessagesParams},
};
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
        print_message(idx, message);
    }

    Ok(())
}

fn print_message(index: usize, message: &ScanMessage) {
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

    if let Some(destination) = &message.destination {
        let dest_status = destination_status(destination).unwrap_or_else(|| "-".to_string());
        let dest_tx = destination_tx(destination).unwrap_or_else(|| "-".to_string());
        let dest_block = destination_block(destination);
        println!("  Destination status : {}", dest_status);
        println!("  Destination tx     : {}", dest_tx);
        if let Some(block) = dest_block {
            println!("  Destination block  : {}", block);
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
}
