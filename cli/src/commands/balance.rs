use alloy::primitives::B256;
use anyhow::{Context, Result};
use client_common::{
    contracts::{erc20::Erc20Contract, utils::get_address_from_private_key},
    tokens::TokenEntry,
};

use crate::{
    BalanceArgs,
    commands::shared::{build_erc20, build_liquidity_manager, find_token_by_chain},
};

pub async fn run(args: &BalanceArgs, tokens: &[TokenEntry], private_key: B256) -> Result<()> {
    let entry = find_token_by_chain(tokens, args.chain_id)?;
    let account = get_address_from_private_key(private_key);

    let zerc20 = build_erc20(entry)?;
    let zerc20_balance = zerc20
        .balance_of(account)
        .await
        .with_context(|| format!("failed to fetch zERC20 balance for {}", entry.label))?;

    let liquidity_manager = build_liquidity_manager(entry)?;
    let underlying_address = liquidity_manager
        .underlying_token()
        .await
        .context("failed to fetch underlying token address")?;
    let underlying = Erc20Contract::new(liquidity_manager.provider(), underlying_address);
    let underlying_balance = underlying.balance_of(account).await.with_context(|| {
        format!(
            "failed to fetch underlying token balance at {}",
            underlying_address
        )
    })?;

    println!("Account             : {}", account);
    println!("  Chain ID          : {}", entry.chain_id);
    println!("  label             : {}", entry.label);
    println!("  zERC20 balance    : {}", zerc20_balance);
    println!("  Underlying balance: {}", underlying_balance);

    Ok(())
}
