use alloy::primitives::B256;
use anyhow::{Context, Result};
use client_common::{contracts::utils::get_address_from_private_key, tokens::TokenEntry};

use crate::{
    TransferArgs,
    commands::shared::{build_erc20, find_token_by_chain, format_tx_hash},
};

pub async fn run(args: &TransferArgs, tokens: &[TokenEntry], private_key: B256) -> Result<()> {
    let entry = find_token_by_chain(tokens, args.chain_id)?;
    let erc20 = build_erc20(entry)?;

    let sender = get_address_from_private_key(private_key);
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
