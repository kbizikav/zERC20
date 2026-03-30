use alloy::{
    network::EthereumWallet,
    primitives::{Address, B256, Bytes, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
    sol,
};
use anyhow::{Context, Result};

use client_common::contracts::relay::RelayTeleportRequest;
use client_common::tokens::TokenEntry;

sol! {
    #[sol(rpc)]
    interface IERC20Permit {
        function permit(address owner, address spender, uint256 value, uint256 deadline, uint8 v, bytes32 r, bytes32 s);
        function transferFrom(address from, address to, uint256 amount) returns (bool);
    }

    #[sol(rpc)]
    interface ISwapHelper {
        function swap(
            address token,
            address owner,
            address recipient,
            uint256 tokenAmount,
            uint256 deadline,
            uint8 v,
            bytes32 r,
            bytes32 s
        ) external payable;
    }
}

/// Submit a teleport transaction to the Verifier contract on behalf of a user.
///
/// Uses the Verifier ABI types re-exported from client-common.
///
/// Returns the transaction hash.
pub async fn submit_teleport(
    token: &TokenEntry,
    relayer_key: &B256,
    req: &RelayTeleportRequest,
) -> Result<B256> {
    use client_common::contracts::verifier::{GeneralRecipientLib, Verifier};

    let signer = PrivateKeySigner::from_bytes(relayer_key)
        .context("failed to create signer from relayer private key")?;
    let wallet = EthereumWallet::from(signer);

    let rpc_url = token
        .rpc_urls
        .first()
        .context("token has no rpc urls configured")?;
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(rpc_url.parse().context("invalid RPC URL")?);

    let contract = Verifier::new(token.verifier_address, &provider);

    let gr = GeneralRecipientLib::GeneralRecipient {
        chainId: req.chain_id,
        recipient: req.recipient,
        tweak: req.tweak,
    };
    let fee_auth = Verifier::RelayerFeeAuthorization {
        relayerFee: req.relayer_fee,
        maxFee: req.max_fee,
        deadline: req.deadline,
        signature: Bytes::from(req.signature.clone()),
    };

    let legacy_gas_price = if token.legacy_tx() {
        Some(
            provider
                .get_gas_price()
                .await
                .context("failed to fetch gas price for legacy tx")?,
        )
    } else {
        None
    };

    let pending = if req.is_single {
        let call = contract.singleTeleport_1(
            req.is_global,
            req.root_hint,
            gr,
            Bytes::copy_from_slice(&req.proof),
            fee_auth,
        );
        let call = match legacy_gas_price {
            Some(gp) => call.gas_price(gp),
            None => call,
        };
        call.send()
            .await
            .context("failed to send singleTeleport transaction")?
    } else {
        let call = contract.teleport_1(
            req.is_global,
            req.root_hint,
            gr,
            Bytes::copy_from_slice(&req.proof),
            fee_auth,
        );
        let call = match legacy_gas_price {
            Some(gp) => call.gas_price(gp),
            None => call,
        };
        call.send()
            .await
            .context("failed to send teleport transaction")?
    };

    let tx_hash = *pending.tx_hash();
    log::info!(
        "Submitted teleport tx {} on chain {}",
        tx_hash,
        token.chain_id
    );
    Ok(tx_hash)
}

/// Execute a token-to-native swap on behalf of a user.
///
/// If the token has a `swap_helper_address` configured, uses the SwapHelper contract
/// for an atomic single-transaction swap. Otherwise falls back to the legacy 3-tx flow.
#[allow(clippy::too_many_arguments)]
pub async fn submit_swap(
    token: &TokenEntry,
    relayer_key: &B256,
    owner: Address,
    recipient: Address,
    token_amount: U256,
    native_amount: U256,
    permit_deadline: U256,
    permit_v: u8,
    permit_r: B256,
    permit_s: B256,
) -> Result<B256> {
    if let Some(swap_helper_address) = token.swap_helper_address {
        submit_swap_atomic(
            token,
            relayer_key,
            swap_helper_address,
            owner,
            recipient,
            token_amount,
            native_amount,
            permit_deadline,
            permit_v,
            permit_r,
            permit_s,
        )
        .await
    } else {
        let hashes = submit_swap_legacy(
            token,
            relayer_key,
            owner,
            recipient,
            token_amount,
            native_amount,
            permit_deadline,
            permit_v,
            permit_r,
            permit_s,
        )
        .await?;
        // Return the last tx hash (native transfer) as the canonical result
        Ok(hashes.native_tx_hash)
    }
}

/// Atomic swap via SwapHelper contract (single transaction).
#[allow(clippy::too_many_arguments)]
async fn submit_swap_atomic(
    token: &TokenEntry,
    relayer_key: &B256,
    swap_helper_address: Address,
    owner: Address,
    recipient: Address,
    token_amount: U256,
    native_amount: U256,
    permit_deadline: U256,
    permit_v: u8,
    permit_r: B256,
    permit_s: B256,
) -> Result<B256> {
    let signer = PrivateKeySigner::from_bytes(relayer_key)
        .context("failed to create signer from relayer private key")?;
    let wallet = EthereumWallet::from(signer);

    let rpc_url = token
        .rpc_urls
        .first()
        .context("token has no rpc urls configured")?;
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(rpc_url.parse().context("invalid RPC URL")?);

    let swap_helper = ISwapHelper::new(swap_helper_address, &provider);

    let call = swap_helper
        .swap(
            token.token_address,
            owner,
            recipient,
            token_amount,
            permit_deadline,
            permit_v,
            permit_r,
            permit_s,
        )
        .value(native_amount);

    let call = if token.legacy_tx() {
        let gp = provider
            .get_gas_price()
            .await
            .context("failed to fetch gas price for legacy tx")?;
        call.gas_price(gp)
    } else {
        call
    };

    let pending = call
        .send()
        .await
        .context("failed to send SwapHelper.swap transaction")?;
    let tx_hash = *pending.tx_hash();
    log::info!("Swap atomic tx: {} on chain {}", tx_hash, token.chain_id);

    Ok(tx_hash)
}

/// Legacy 3-transaction swap (permit → transferFrom → native transfer).
#[allow(clippy::too_many_arguments)]
async fn submit_swap_legacy(
    token: &TokenEntry,
    relayer_key: &B256,
    owner: Address,
    recipient: Address,
    token_amount: U256,
    native_amount: U256,
    permit_deadline: U256,
    permit_v: u8,
    permit_r: B256,
    permit_s: B256,
) -> Result<SwapTxHashes> {
    let signer = PrivateKeySigner::from_bytes(relayer_key)
        .context("failed to create signer from relayer private key")?;
    let relayer_address = signer.address();
    let wallet = EthereumWallet::from(signer);

    let rpc_url = token
        .rpc_urls
        .first()
        .context("token has no rpc urls configured")?;
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(rpc_url.parse().context("invalid RPC URL")?);

    let erc20 = IERC20Permit::new(token.token_address, &provider);

    let legacy_gas_price = if token.legacy_tx() {
        Some(
            provider
                .get_gas_price()
                .await
                .context("failed to fetch gas price for legacy tx")?,
        )
    } else {
        None
    };

    // 1. permit
    let permit_call = erc20.permit(
        owner,
        relayer_address,
        token_amount,
        permit_deadline,
        permit_v,
        permit_r,
        permit_s,
    );
    let permit_call = match legacy_gas_price {
        Some(gp) => permit_call.gas_price(gp),
        None => permit_call,
    };
    let permit_pending = permit_call
        .send()
        .await
        .context("failed to send permit transaction")?;
    let permit_tx_hash = *permit_pending.tx_hash();
    log::info!("Swap permit tx: {}", permit_tx_hash);

    // Wait for permit confirmation before proceeding
    let permit_receipt = permit_pending
        .get_receipt()
        .await
        .context("permit transaction failed")?;
    if !permit_receipt.status() {
        anyhow::bail!("permit transaction reverted: {:?}", permit_receipt);
    }

    // 2. transferFrom
    let transfer_call = erc20.transferFrom(owner, relayer_address, token_amount);
    let transfer_call = match legacy_gas_price {
        Some(gp) => transfer_call.gas_price(gp),
        None => transfer_call,
    };
    let transfer_pending = transfer_call
        .send()
        .await
        .context("failed to send transferFrom transaction")?;
    let transfer_tx_hash = *transfer_pending.tx_hash();
    log::info!("Swap transferFrom tx: {}", transfer_tx_hash);

    let transfer_receipt = transfer_pending
        .get_receipt()
        .await
        .context("transferFrom transaction failed")?;
    if !transfer_receipt.status() {
        anyhow::bail!("transferFrom transaction reverted: {:?}", transfer_receipt);
    }

    // 3. Send native tokens to recipient
    let mut tx_req = TransactionRequest::default()
        .to(recipient)
        .value(native_amount);
    if let Some(gp) = legacy_gas_price {
        tx_req = tx_req.gas_price(gp);
    }
    let native_pending = provider
        .send_transaction(tx_req)
        .await
        .context("failed to send native transfer transaction")?;
    let native_tx_hash = *native_pending.tx_hash();
    log::info!("Swap native transfer tx: {}", native_tx_hash);

    Ok(SwapTxHashes {
        permit_tx_hash,
        transfer_tx_hash,
        native_tx_hash,
    })
}

/// Transaction hashes returned from a legacy swap execution.
#[allow(dead_code)]
struct SwapTxHashes {
    permit_tx_hash: B256,
    transfer_tx_hash: B256,
    native_tx_hash: B256,
}
