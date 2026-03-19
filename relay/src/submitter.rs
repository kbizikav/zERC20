use alloy::{
    network::EthereumWallet,
    primitives::{B256, Bytes},
    providers::ProviderBuilder,
    signers::local::PrivateKeySigner,
};
use anyhow::{Context, Result};

use client_common::contracts::relay::RelayTeleportRequest;
use crate::config::ChainConfig;

/// Submit a teleport transaction to the Verifier contract on behalf of a user.
///
/// Uses the Verifier ABI types re-exported from client-common.
///
/// Returns the transaction hash.
pub async fn submit_teleport(
    chain: &ChainConfig,
    relayer_key: &B256,
    req: &RelayTeleportRequest,
) -> Result<B256> {
    use client_common::contracts::verifier::{Verifier, GeneralRecipientLib};

    let signer = PrivateKeySigner::from_bytes(relayer_key)
        .context("failed to create signer from relayer private key")?;
    let wallet = EthereumWallet::from(signer);

    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(chain.rpc_url.parse().context("invalid RPC URL")?);

    let contract = Verifier::new(chain.verifier_address, &provider);

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

    // Determine if this is a single (Groth16) or batch (Nova) proof based on
    // proof size: Groth16 proofs are exactly 256 bytes (8 × 32-byte elements).
    let is_single = req.proof.len() == 256;

    let pending = if is_single {
        contract
            .singleTeleport_1(
                req.is_global,
                req.root_hint,
                gr,
                Bytes::copy_from_slice(&req.proof),
                fee_auth,
            )
            .send()
            .await
            .context("failed to send singleTeleport transaction")?
    } else {
        contract
            .teleport_1(
                req.is_global,
                req.root_hint,
                gr,
                Bytes::copy_from_slice(&req.proof),
                fee_auth,
            )
            .send()
            .await
            .context("failed to send teleport transaction")?
    };

    let tx_hash = *pending.tx_hash();
    log::info!(
        "Submitted teleport tx {} on chain {}",
        tx_hash,
        chain.chain_id
    );
    Ok(tx_hash)
}
