use alloy::primitives::{B256, Bytes};
use anyhow::{Context, Result, bail, ensure};
use client_common::{
    contracts::{
        adaptor::{AdaptorContract, BridgeRequest},
        utils::get_address_from_private_key,
    },
    tokens::TokenEntry,
};

use crate::{QuoteUnwrapArgs, commands::shared::find_token_by_chain};

const WORKER_ID_EXECUTOR: u8 = 1;
const OPTION_TYPE_LZ_RECEIVE: u8 = 1;
const OPTIONS_TYPE_3: u16 = 3;

pub async fn run(args: &QuoteUnwrapArgs, tokens: &[TokenEntry], private_key: B256) -> Result<()> {
    let caller = get_address_from_private_key(private_key);
    let receiver = args.receiver.unwrap_or(caller);
    let refund_address = args.refund_address.unwrap_or(caller);
    let min_amount_out = args.min_amount_out.unwrap_or(args.amount);

    let dst_entry = find_token_by_chain(tokens, args.dst_chain_id)
        .with_context(|| format!("failed to resolve destination chain {}", args.dst_chain_id))?;
    let dst_eid = dst_entry
        .eid
        .with_context(|| format!("destination chain {} is missing an eid", args.dst_chain_id))?;

    let extra_options = build_extra_options(args.lz_receive_gas, args.lz_receive_value)?;
    let compose_msg = args.compose_msg.clone().unwrap_or_default();
    let oft_cmd = args.oft_cmd.clone().unwrap_or_default();

    let mut quoted_any = false;
    for entry in tokens
        .iter()
        .filter(|token| token.adaptor_address.is_some())
    {
        quoted_any = true;
        let adaptor_address = entry
            .adaptor_address
            .expect("checked adaptor presence by filter");
        let provider = entry
            .provider()
            .with_context(|| format!("failed to construct provider for '{}'", entry.label))?;
        let adaptor =
            AdaptorContract::new(provider, adaptor_address).with_legacy_tx(entry.legacy_tx);

        let request = BridgeRequest {
            dst_eid,
            extra_options: extra_options.clone(),
            compose_msg: compose_msg.clone(),
            oft_cmd: oft_cmd.clone(),
            refund_address,
            to: receiver,
            min_amount_out,
        };

        println!("Token label         : {}", entry.label);
        println!("Chain ID            : {}", entry.chain_id);
        println!("Adaptor address     : {}", adaptor.address());
        println!(
            "Destination chain   : {} (eid {})",
            dst_entry.label, dst_eid
        );
        println!("Receiver            : {}", receiver);
        println!("Refund address      : {}", refund_address);
        println!("Amount              : {}", args.amount);

        let quote = adaptor
            .quote_fee(args.amount, request)
            .await
            .with_context(|| format!("failed to quote unwrap + bridge on '{}'", entry.label))?;

        println!("  Token unwrap fee  : {}", quote.token_unwrap_fee);
        println!("  Native bridge fee : {}", quote.native_bridge_fee);
        println!("  Token bridge fee  : {}", quote.token_bridge_fee);
        println!();
    }

    if !quoted_any {
        bail!("no adaptor addresses configured in tokens.json");
    }

    Ok(())
}

fn build_extra_options(lz_receive_gas: u32, lz_receive_value: u128) -> Result<Bytes> {
    let gas = u128::from(lz_receive_gas);
    let gas_bytes = gas.to_be_bytes();
    let mut option = Vec::from(gas_bytes);
    if lz_receive_value > 0 {
        option.extend_from_slice(&lz_receive_value.to_be_bytes());
    }

    let option_size = option
        .len()
        .checked_add(1)
        .context("failed to compute executor option size")?;
    ensure!(
        option_size <= u16::MAX as usize,
        "executor option length {} exceeds u16::MAX",
        option_size
    );

    let mut options = Vec::new();
    options.extend_from_slice(&OPTIONS_TYPE_3.to_be_bytes());
    options.push(WORKER_ID_EXECUTOR);
    options.extend_from_slice(&(option_size as u16).to_be_bytes());
    options.push(OPTION_TYPE_LZ_RECEIVE);
    options.extend_from_slice(&option);

    Ok(Bytes::from(options))
}
