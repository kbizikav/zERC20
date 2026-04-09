// SPDX-License-Identifier: BUSL-1.1

use alloy::{
    network::Ethereum,
    primitives::{Address, B256},
    providers::{PendingTransactionBuilder, Provider as _},
};
use anyhow::{Context, Result, bail};
use client_common::{
    contracts::{erc20::Erc20Contract, utils::get_address_from_private_key},
    tokens::TokenEntry,
};
use std::str::FromStr;

use crate::{
    WrapArgs,
    commands::shared::{build_liquidity_manager, find_token_by_chain, format_tx_hash},
};

const NATIVE_TOKEN: &str = "0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE";

pub async fn run(args: &WrapArgs, tokens: &[TokenEntry], private_key: B256) -> Result<()> {
    let entry = find_token_by_chain(tokens, args.chain_id)?;
    let manager = build_liquidity_manager(entry)?;
    let caller = get_address_from_private_key(private_key);
    let receiver = args.receiver.unwrap_or(caller);

    let underlying_address = manager
        .underlying_token()
        .await
        .context("failed to fetch underlying token address")?;
    if underlying_address == Address::ZERO {
        bail!(
            "liquidity manager {} is not configured with an underlying token",
            manager.address()
        );
    }

    println!("Token label         : {}", entry.label);
    println!("Liquidity manager   : {}", manager.address());
    println!("Receiver address    : {}", receiver);
    println!("Amount              : {}", args.amount);

    let native_token = Address::from_str(NATIVE_TOKEN).expect("invalid native token address");
    let pending = if underlying_address == native_token {
        let balance = manager
            .provider()
            .get_balance(caller)
            .await
            .context("failed to fetch native balance")?;
        if balance < args.amount {
            bail!(
                "insufficient native balance: have {}, need {}",
                balance,
                args.amount
            );
        }
        manager
            .wrap_with_value(private_key, args.amount, receiver, args.amount)
            .await
            .context("failed to submit native wrap transaction")?
    } else {
        let underlying = Erc20Contract::new(manager.provider(), underlying_address)
            .with_legacy_tx(entry.legacy_tx);
        let balance = underlying.balance_of(caller).await.with_context(|| {
            format!(
                "failed to fetch underlying balance at {}",
                underlying_address
            )
        })?;

        if balance < args.amount {
            bail!(
                "insufficient underlying token balance: have {}, need {}",
                balance,
                args.amount
            );
        }

        let allowance = underlying
            .allowance(caller, manager.address())
            .await
            .context("failed to fetch allowance for liquidity manager")?;

        if allowance < args.amount {
            let approval_pending = underlying
                .approve(private_key, manager.address(), args.amount)
                .await
                .context("failed to submit approval transaction")?;
            let approval_hash = format_tx_hash(approval_pending.tx_hash().as_slice());
            println!("Submitted approval   : {}", approval_hash);
            wait_for_receipt(approval_pending).await?;
        } else {
            println!(
                "Skipping approval    : existing allowance {} >= {}",
                allowance, args.amount
            );
        }

        manager
            .wrap(private_key, args.amount, receiver)
            .await
            .context("failed to submit wrap transaction")?
    };
    let wrap_hash = format_tx_hash(pending.tx_hash().as_slice());
    println!("Submitted wrap       : {}", wrap_hash);

    let receipt = wait_for_receipt(pending).await?;
    let wrapped = manager
        .parse_wrapped(&receipt)
        .context("failed to parse Wrapped event from receipt")?;

    println!("Wrapped amount       : {}", wrapped.amount_out);

    Ok(())
}

async fn wait_for_receipt(
    pending: PendingTransactionBuilder<Ethereum>,
) -> Result<alloy::rpc::types::TransactionReceipt> {
    let receipt = pending
        .get_receipt()
        .await
        .context("failed to fetch transaction receipt")?;
    if receipt.status() {
        Ok(receipt)
    } else {
        bail!("transaction reverted: {:?}", receipt);
    }
}
