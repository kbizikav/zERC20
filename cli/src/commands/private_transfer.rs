use alloy::primitives::B256;
use anyhow::{Context, Result};
use client_common::{
    payment::{
        burn_address::FullBurnAddress,
        invoice::{SecretAndTweak, random_payment_advice_id},
        seed::compute_seed_from_signature,
    },
    tokens::TokenEntry,
};
use rand::rngs::OsRng;
use stealth_client::encryption::encrypt_payload;

use crate::{
    CommonArgs, PrivateTransferArgs,
    commands::shared::{build_erc20, build_stealth_client, find_token_by_chain, format_tx_hash},
};
use hex;

pub async fn run(
    common: &CommonArgs,
    args: &PrivateTransferArgs,
    tokens: &[TokenEntry],
    private_key: B256,
) -> Result<()> {
    let entry = find_token_by_chain(tokens, args.chain_id)?;
    let erc20 = build_erc20(entry)?;

    let payment_advice_id = random_payment_advice_id();
    let seed = compute_seed_from_signature(private_key)
        .await
        .context("failed to derive seed from signature")?;
    let secret_and_tweak =
        SecretAndTweak::payment_advice(payment_advice_id, seed, args.to_chain_id, args.to);
    let burn_payload = FullBurnAddress::new(args.to_chain_id, args.to, &secret_and_tweak)?;
    let burn_address = burn_payload
        .burn_address()
        .context("failed to derive burn address for private transfer")?;
    let burn_payload_bytes = burn_payload.to_bytes();

    println!("Token label        : {}", entry.label);
    println!("Token address      : {}", entry.token_address);
    println!("Recipient address  : {}", args.to);
    println!("Recipient chain ID : {}", args.to_chain_id);
    println!(
        "Payment advice ID  : 0x{}",
        hex::encode(payment_advice_id.as_slice())
    );
    println!("Burn address       : {}", burn_address);
    println!(
        "FullBurnAddress    : 0x{}",
        hex::encode(&burn_payload_bytes)
    );
    println!("Amount (raw)       : {}", args.amount);

    let client = build_stealth_client(common)
        .await
        .context("failed to construct stealth canister client")?;

    let mut recipient_bytes = [0u8; 20];
    recipient_bytes.copy_from_slice(args.to.as_slice());

    let view_public_key = client
        .get_view_public_key(recipient_bytes)
        .await
        .context("failed to query view public key for recipient")?;

    let mut rng = OsRng;
    let announcement_input = encrypt_payload(&mut rng, &view_public_key, &burn_payload_bytes)
        .context("failed to encrypt FullBurnAddress payload")?;

    let announcement = client
        .submit_announcement(&announcement_input)
        .await
        .context("failed to submit encrypted FullBurnAddress to storage canister")?;

    println!("Storage announcement : {}", announcement.id);

    let pending = erc20
        .transfer(private_key, burn_address, args.amount)
        .await
        .with_context(|| format!("failed to submit private transfer for {}", entry.label))?;

    let tx_hash = format_tx_hash(pending.tx_hash().as_slice());
    println!("Submitted transfer  : {}", tx_hash);

    Ok(())
}
