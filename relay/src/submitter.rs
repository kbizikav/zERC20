use alloy::{
    network::EthereumWallet,
    primitives::{B256, Bytes},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use anyhow::{Context, Result};

use client_common::contracts::relay::RelayTeleportRequest;
use client_common::tokens::TokenEntry;

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
