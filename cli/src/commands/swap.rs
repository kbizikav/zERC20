use alloy::primitives::{Address, B256, U256, keccak256};
use alloy::signers::{Signer, local::PrivateKeySigner};
use anyhow::{Context, Result, bail};
use client_common::contracts::relay::{
    RelaySwapRequest, fetch_relay_info, fetch_swap_quote, submit_relay_swap,
};
use client_common::contracts::utils::get_address_from_private_key;
use client_common::tokens::TokenEntry;

use super::shared::{build_erc20, find_token_by_chain};

/// Execute a token-to-native swap via the relay node.
///
/// 1. Fetch relay info (address).
/// 2. Fetch a quote from the relay.
/// 3. Sign an ERC-2612 permit granting the relay allowance.
/// 4. Submit the swap request.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    tokens: &[TokenEntry],
    private_key: B256,
    chain_id: u64,
    amount: U256,
    relay_url: &str,
    slippage_bps: u64,
    recipient: Option<Address>,
    yes: bool,
) -> Result<()> {
    let token = find_token_by_chain(tokens, chain_id)?;
    let owner = get_address_from_private_key(private_key);
    let recipient = recipient.unwrap_or(owner);

    // Fetch relay address (needed for permit spender)
    let relay_info = fetch_relay_info(relay_url)
        .await
        .context("failed to fetch relay info")?;
    if !relay_info.swap_enabled {
        bail!("swap is not enabled on this relay node");
    }
    // Use SwapHelper address as permit spender if available, otherwise fall back to relayer
    let spender = relay_info
        .swap_helper_addresses
        .as_ref()
        .and_then(|m| m.get(&chain_id.to_string()).copied())
        .unwrap_or(relay_info.address);
    println!("Relay address: {}", relay_info.address);
    if spender != relay_info.address {
        println!("SwapHelper   : {}", spender);
    }

    // Check balance
    let erc20 = build_erc20(token)?;
    let balance = erc20
        .balance_of(owner)
        .await
        .context("failed to fetch zERC20 balance")?;
    if balance < amount {
        bail!(
            "insufficient zERC20 balance: have {}, need {}",
            balance,
            amount
        );
    }

    // 1. Get swap quote
    println!("Fetching swap quote...");
    let quote = fetch_swap_quote(relay_url, chain_id, amount)
        .await
        .context("failed to fetch swap quote")?;

    let native_amount =
        U256::from_str_radix(&quote.native_amount, 10).context("invalid native_amount in quote")?;
    let relayer_fee =
        U256::from_str_radix(&quote.relayer_fee, 10).context("invalid relayer_fee in quote")?;

    println!("  Token amount   : {}", amount);
    println!("  Native output  : {} wei", native_amount);
    println!("  Fee            : {} bps", quote.fee_bps);
    println!("  Relayer fee    : {} wei", relayer_fee);
    if quote.price_fallback {
        println!("  WARNING: relay is using fallback/stale oracle prices — quoted rate may be less favorable");
    }

    // Apply slippage to compute min_native_amount
    let min_native = native_amount * U256::from(10_000 - slippage_bps) / U256::from(10_000u64);
    println!(
        "  Min native out : {} wei ({}bps slippage)",
        min_native, slippage_bps
    );

    if native_amount.is_zero() {
        bail!("swap quote returned zero native output");
    }

    if !yes {
        print!("\nProceed with swap? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Swap cancelled.");
            return Ok(());
        }
    }

    // 2. Sign ERC-2612 permit
    println!("Signing ERC-2612 permit...");

    let deadline = U256::from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 1800, // 30 minutes
    );

    let nonce = erc20
        .nonces(owner)
        .await
        .context("failed to fetch permit nonce")?;

    // Fetch EIP-712 domain from the token contract
    let domain = erc20
        .eip712_domain()
        .await
        .context("failed to fetch eip712Domain from token")?;

    let (v, r, s) = sign_eip2612_permit(
        private_key,
        &domain.name,
        &domain.version,
        domain.chain_id,
        domain.verifying_contract,
        owner,
        spender,
        amount,
        nonce,
        deadline,
    )
    .await
    .context("failed to sign permit")?;

    // 3. Submit swap
    println!("Submitting swap to relay...");
    let req = RelaySwapRequest {
        chain_id,
        token_amount: amount.to_string(),
        min_native_amount: min_native.to_string(),
        recipient,
        owner,
        permit_deadline: deadline.to_string(),
        permit_v: v,
        permit_r: r,
        permit_s: s,
    };

    let result = submit_relay_swap(relay_url, &req)
        .await
        .context("swap submission failed")?;

    println!("\nSwap completed!");
    println!("  Tx hash: {}", result.tx_hash);

    Ok(())
}

/// Sign an EIP-2612 permit (ERC20Permit standard).
#[allow(clippy::too_many_arguments)]
async fn sign_eip2612_permit(
    private_key: B256,
    name: &str,
    version: &str,
    chain_id: U256,
    verifying_contract: Address,
    owner: Address,
    spender: Address,
    value: U256,
    nonce: U256,
    deadline: U256,
) -> Result<(u8, B256, B256)> {
    // Domain separator
    let domain_type_hash = keccak256(
        "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let mut domain_data = Vec::with_capacity(5 * 32);
    domain_data.extend_from_slice(domain_type_hash.as_slice());
    domain_data.extend_from_slice(keccak256(name.as_bytes()).as_slice());
    domain_data.extend_from_slice(keccak256(version.as_bytes()).as_slice());
    domain_data.extend_from_slice(&chain_id.to_be_bytes::<32>());
    domain_data
        .extend_from_slice(B256::left_padding_from(verifying_contract.as_slice()).as_slice());
    let domain_separator = keccak256(&domain_data);

    // Struct hash
    let permit_type_hash = keccak256(
        "Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)",
    );
    let mut struct_data = Vec::with_capacity(6 * 32);
    struct_data.extend_from_slice(permit_type_hash.as_slice());
    struct_data.extend_from_slice(B256::left_padding_from(owner.as_slice()).as_slice());
    struct_data.extend_from_slice(B256::left_padding_from(spender.as_slice()).as_slice());
    struct_data.extend_from_slice(&value.to_be_bytes::<32>());
    struct_data.extend_from_slice(&nonce.to_be_bytes::<32>());
    struct_data.extend_from_slice(&deadline.to_be_bytes::<32>());
    let struct_hash = keccak256(&struct_data);

    // EIP-712 digest
    let mut digest_input = Vec::with_capacity(2 + 32 + 32);
    digest_input.extend_from_slice(&[0x19, 0x01]);
    digest_input.extend_from_slice(domain_separator.as_slice());
    digest_input.extend_from_slice(struct_hash.as_slice());
    let digest = keccak256(&digest_input);

    let signer = PrivateKeySigner::from_bytes(&private_key).context("failed to create signer")?;
    let sig = signer
        .sign_hash(&digest)
        .await
        .context("failed to sign permit")?;

    let bytes = sig.as_bytes();
    let v = bytes[64];
    let r = B256::from_slice(&bytes[..32]);
    let s = B256::from_slice(&bytes[32..64]);

    Ok((v, r, s))
}
