use alloy::{
    network::Ethereum,
    primitives::{Address, B256, Bytes, U256},
    providers::{PendingTransactionBuilder, Provider as _},
    sol_types::SolValue,
};
use anyhow::{Context, Result, bail, ensure};
use client_common::{
    contracts::{
        adaptor::{Adaptor, AdaptorContract, BridgeRequest},
        erc20::Erc20Contract,
        utils::get_address_from_private_key,
        z_erc20::{SendParam, ZErc20Contract},
    },
    tokens::TokenEntry,
};

use crate::{
    UnwrapArgs,
    commands::{
        quote_unwrap::build_extra_options,
        shared::{build_erc20, build_liquidity_manager, find_token_by_chain, format_tx_hash},
    },
};

const RECEIVE_GAS: u32 = 200_000;
const COMPOSE_GAS: u32 = 600_000;

pub async fn run(args: &UnwrapArgs, tokens: &[TokenEntry], private_key: B256) -> Result<()> {
    let caller = get_address_from_private_key(private_key);
    let src_entry = find_token_by_chain(tokens, args.chain_id)?;
    let zerc20 = build_erc20(src_entry)?;

    if args.amount.is_zero() {
        bail!("amount must be greater than zero");
    }

    let balance = zerc20
        .balance_of(caller)
        .await
        .context("failed to fetch zERC20 balance")?;
    if balance < args.amount {
        bail!(
            "insufficient zERC20 balance: have {}, need {}",
            balance,
            args.amount
        );
    }

    if args.dst_chain_id == args.chain_id {
        unwrap_local(src_entry, caller, args.amount, private_key).await
    } else {
        let dst_entry = find_token_by_chain(tokens, args.dst_chain_id).with_context(|| {
            format!("failed to resolve destination chain {}", args.dst_chain_id)
        })?;
        unwrap_cross_chain(
            src_entry,
            dst_entry,
            caller,
            args.amount,
            private_key,
            &zerc20,
        )
        .await
    }
}

async fn unwrap_local(
    entry: &TokenEntry,
    caller: Address,
    amount: U256,
    private_key: B256,
) -> Result<()> {
    let manager = build_liquidity_manager(entry)?;
    let provider = entry.provider()?;
    let zerc20 = Erc20Contract::new(provider, entry.token_address).with_legacy_tx(entry.legacy_tx);

    println!("Token label         : {}", entry.label);
    println!("Chain ID            : {}", entry.chain_id);
    println!("Caller address      : {}", caller);
    println!("Amount              : {}", amount);
    println!("Liquidity manager   : {}", manager.address());
    println!("zERC20 address      : {}", entry.token_address);

    let allowance = zerc20
        .allowance(caller, manager.address())
        .await
        .context("failed to fetch allowance for liquidity manager")?;
    if allowance < amount {
        let approval_pending = zerc20
            .approve(private_key, manager.address(), amount)
            .await
            .context("failed to submit approval transaction")?;
        let approval_hash = format_tx_hash(approval_pending.tx_hash().as_slice());
        println!("Submitted approval   : {}", approval_hash);
        wait_for_receipt(approval_pending).await?;
    } else {
        println!(
            "Skipping approval    : existing allowance {} >= {}",
            allowance, amount
        );
    }

    let pending = manager
        .unwrap(private_key, amount, caller)
        .await
        .context("failed to submit unwrap transaction")?;
    let tx_hash = format_tx_hash(pending.tx_hash().as_slice());
    println!("Submitted unwrap     : {}", tx_hash);

    let receipt = wait_for_receipt(pending).await?;
    let unwrapped = manager
        .parse_unwrapped(&receipt)
        .context("failed to parse Unwrapped event from receipt")?;

    println!("Unwrapped amount     : {}", unwrapped.amount_out);
    println!("Unwrap fee paid      : {}", unwrapped.fee_amount);

    Ok(())
}

async fn unwrap_cross_chain(
    src_entry: &TokenEntry,
    dst_entry: &TokenEntry,
    caller: Address,
    amount: U256,
    private_key: B256,
    zerc20: &ZErc20Contract,
) -> Result<()> {
    let adaptor_address = dst_entry
        .adaptor_address
        .with_context(|| format!("token '{}' is missing an adaptor address", dst_entry.label))?;
    let src_eid = src_entry
        .eid
        .with_context(|| format!("source chain {} is missing an eid", src_entry.chain_id))?;
    let dst_eid = dst_entry
        .eid
        .with_context(|| format!("destination chain {} is missing an eid", dst_entry.chain_id))?;

    let dst_provider = dst_entry.provider()?;
    let adaptor =
        AdaptorContract::new(dst_provider, adaptor_address).with_legacy_tx(dst_entry.legacy_tx);

    println!(
        "Source chain        : {} ({})",
        src_entry.label, src_entry.chain_id
    );
    println!(
        "Destination chain   : {} ({})",
        dst_entry.label, dst_entry.chain_id
    );
    println!("Destination adaptor : {}", adaptor.address());
    println!("Caller address      : {}", caller);
    println!("Amount              : {}", amount);

    let return_extra_options = build_extra_options(RECEIVE_GAS, 0, &[])
        .context("failed to build initial bridge extra options")?;
    let return_bridge_request = BridgeRequest {
        dst_eid: src_eid,
        to: caller,
        min_amount_out: U256::ZERO,
        extra_options: return_extra_options,
        compose_msg: Bytes::new(),
        oft_cmd: Bytes::new(),
    };

    let return_fee_quote = adaptor
        .quote_fee(amount, return_bridge_request.clone())
        .await
        .context("failed to quote unwrap on destination adaptor")?;
    let native_bridge_fee_with_buffer = return_fee_quote
        .native_bridge_fee
        .checked_mul(U256::from(3))
        .and_then(|v| v.checked_div(U256::from(2)))
        .context("failed to scale native bridge fee by 1.5x")?;
    ensure!(
        native_bridge_fee_with_buffer <= U256::from(u128::MAX),
        "scaled native bridge fee {} exceeds u128::MAX",
        native_bridge_fee_with_buffer
    );
    let native_bridge_fee_with_buffer_u128: u128 = native_bridge_fee_with_buffer
        .try_into()
        .expect("scaled fee checked to fit into u128");
    let compose_options = vec![(COMPOSE_GAS, native_bridge_fee_with_buffer_u128)];
    let extra_options = build_extra_options(RECEIVE_GAS, 0, &compose_options)
        .context("failed to build extra options")?;

    let fee_quote = adaptor
        .quote_fee(amount, return_bridge_request.clone())
        .await
        .context("failed to quote unwrap on destination adaptor")?;
    let token_fee = fee_quote
        .token_bridge_fee
        .saturating_add(fee_quote.token_unwrap_fee);

    if amount <= token_fee {
        bail!(
            "amount {} must exceed unwrap + bridge token fees {}",
            amount,
            token_fee
        );
    }

    let compose_payload = Adaptor::BridgeRequest::from(return_bridge_request.clone()).abi_encode();

    let send_param = SendParam {
        dst_eid,
        to: address_to_b256(adaptor.address()),
        amount_ld: amount,
        min_amount_ld: amount,
        extra_options,
        compose_msg: Bytes::from(compose_payload),
        oft_cmd: Bytes::new(),
    };

    let send_fee = zerc20
        .quote_send(send_param.clone())
        .await
        .context("failed to quote zERC20 send fee")?;
    if send_fee.lz_token_fee > U256::ZERO {
        bail!(
            "lzTokenFee payments are unsupported for adaptor compose (lz_token_fee={})",
            send_fee.lz_token_fee
        );
    }

    let native_balance = src_entry
        .provider()?
        .get_balance(caller)
        .await
        .context("failed to fetch native balance for send fee")?;
    ensure!(
        native_balance > send_fee.native_fee,
        "insufficient native balance: have {}, need more than {}",
        native_balance,
        send_fee.native_fee
    );

    println!("Token unwrap fee    : {}", fee_quote.token_unwrap_fee);
    println!("Token bridge fee    : {}", fee_quote.token_bridge_fee);
    println!(
        "Minimum amount out  : {}",
        return_bridge_request.min_amount_out
    );
    println!("Native send fee     : {}", send_fee.native_fee);

    let pending = zerc20
        .send(private_key, send_param, send_fee, caller)
        .await
        .context("failed to submit zERC20 send to adaptor")?;
    let tx_hash = format_tx_hash(pending.tx_hash().as_slice());
    println!("Submitted send       : {}", tx_hash);

    wait_for_receipt(pending).await?;

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

fn address_to_b256(address: Address) -> B256 {
    let mut padded = [0u8; 32];
    padded[12..].copy_from_slice(address.as_slice());
    B256::from(padded)
}
