use alloy::{
    primitives::{Address, B256, Bytes, U256},
    providers::Provider,
    sol,
};
use anyhow::{Context, Result, anyhow};

use client_common::contracts::relay::RelayTeleportRequest;
use client_common::contracts::utils::ProviderWithSigner;
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
    provider: &ProviderWithSigner,
    req: &RelayTeleportRequest,
) -> Result<B256> {
    use client_common::contracts::verifier::{GeneralRecipientLib, Verifier};

    let contract = Verifier::new(token.verifier_address, provider);

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
/// Requires the token to have a `swap_helper_address` configured and executes
/// the swap atomically through the SwapHelper contract.
#[allow(clippy::too_many_arguments)]
pub async fn submit_swap(
    token: &TokenEntry,
    provider: Option<&ProviderWithSigner>,
    owner: Address,
    recipient: Address,
    token_amount: U256,
    native_amount: U256,
    permit_deadline: U256,
    permit_v: u8,
    permit_r: B256,
    permit_s: B256,
) -> Result<B256> {
    let provider = provider.ok_or_else(|| {
        anyhow!(
            "no signer provider configured for chain {} ({})",
            token.chain_id,
            token.label
        )
    })?;
    let swap_helper_address = token.swap_helper_address.ok_or_else(|| {
        anyhow!(
            "no swap_helper_address configured for chain {} ({})",
            token.chain_id,
            token.label
        )
    })?;
    submit_swap_atomic(
        token,
        provider,
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
}

/// Atomic swap via SwapHelper contract (single transaction).
#[allow(clippy::too_many_arguments)]
async fn submit_swap_atomic(
    token: &TokenEntry,
    provider: &ProviderWithSigner,
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
    let swap_helper = ISwapHelper::new(swap_helper_address, provider);

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
