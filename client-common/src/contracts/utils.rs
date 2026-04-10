// SPDX-License-Identifier: BUSL-1.1

use alloy::{
    consensus::Transaction as _,
    contract::{CallBuilder, CallDecoder},
    network::{Ethereum, EthereumWallet},
    primitives::{Address, B256, U256},
    providers::{
        Identity, PendingTransactionBuilder, Provider, ProviderBuilder,
        fillers::{
            ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller, SimpleNonceManager,
            WalletFiller,
        },
    },
    rpc::client::RpcClient,
    signers::local::PrivateKeySigner,
    transports::{
        http::Http,
        layers::{FallbackLayer, RetryBackoffLayer},
    },
};
use anyhow::Context;
use hex;
use reqwest::Url;
use std::{str::FromStr, time::Duration};
use tower::ServiceBuilder;

use crate::contracts::{ContractError, ContractResult};

pub type JoinedRecommendedFillersWithSimpleNonce = JoinFill<
    JoinFill<JoinFill<Identity, GasFiller>, NonceFiller<SimpleNonceManager>>,
    ChainIdFiller,
>;

pub type NormalProvider =
    FillProvider<JoinedRecommendedFillersWithSimpleNonce, alloy::providers::RootProvider>;

pub type ProviderWithSigner = FillProvider<
    JoinFill<JoinedRecommendedFillersWithSimpleNonce, WalletFiller<EthereumWallet>>,
    alloy::providers::RootProvider,
>;

/// Default timeout for individual RPC HTTP requests.
const RPC_REQUEST_TIMEOUT_SECS: u64 = 60;
/// Default connection timeout for RPC HTTP connections.
const RPC_CONNECT_TIMEOUT_SECS: u64 = 30;

fn rpc_http_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(RPC_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(RPC_REQUEST_TIMEOUT_SECS))
        .build()
        .context("failed to build RPC HTTP client")
}

pub fn get_provider(rpc_url: &str) -> anyhow::Result<NormalProvider> {
    let retry_layer = RetryBackoffLayer::new(5, 1000, 100);
    let url: Url = rpc_url
        .parse()
        .context(format!("Failed to parse rpc url: {}", rpc_url))?;
    let http = Http::with_client(rpc_http_client()?, url);
    let client = RpcClient::builder()
        .layer(retry_layer)
        .transport(http, false);
    let provider = ProviderBuilder::default()
        .with_gas_estimation()
        .with_simple_nonce_management()
        .fetch_chain_id()
        .connect_client(client);
    Ok(provider)
}

pub fn get_provider_with_fallback(rpc_urls: &[String]) -> anyhow::Result<NormalProvider> {
    let retry_layer = RetryBackoffLayer::new(5, 1000, 100);
    let http_client = rpc_http_client()?;
    let transports = rpc_urls
        .iter()
        .map(|url| {
            let url: Url = url
                .parse()
                .context(format!("Failed to parse rpc url: {}", url))?;
            Ok(Http::with_client(http_client.clone(), url))
        })
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    let fallback_layer =
        FallbackLayer::default().with_active_transport_count(transports.len().try_into().unwrap());
    let transport = ServiceBuilder::new()
        .layer(fallback_layer)
        .service(transports);
    let client = RpcClient::builder()
        .layer(retry_layer)
        .transport(transport, false);
    let provider = ProviderBuilder::default()
        .with_gas_estimation()
        .with_simple_nonce_management()
        .fetch_chain_id()
        .connect_client(client);
    Ok(provider)
}

pub fn get_provider_with_signer(
    provider: &NormalProvider,
    private_key: B256,
) -> ProviderWithSigner {
    let signer = PrivateKeySigner::from_bytes(&private_key).unwrap();
    let wallet = EthereumWallet::new(signer);
    let wallet_filler = WalletFiller::new(wallet);
    provider.clone().join_with(wallet_filler)
}

pub fn get_address_from_private_key(private_key: B256) -> Address {
    let signer = PrivateKeySigner::from_bytes(&private_key).unwrap();
    signer.address()
}

pub fn uint256_as_u64(value: U256) -> u64 {
    let bytes: [u8; 32] = value.to_be_bytes();
    u64::from_be_bytes(bytes[24..32].try_into().unwrap())
}

pub async fn send_call_with_legacy<D>(
    call: CallBuilder<ProviderWithSigner, D, Ethereum>,
    provider: &ProviderWithSigner,
    use_legacy: bool,
) -> ContractResult<PendingTransactionBuilder<Ethereum>>
where
    D: CallDecoder,
{
    let call = if use_legacy {
        let gas_price = provider.get_gas_price().await.map_err(|err| {
            ContractError::transport("fetching gas price for legacy transfer", err)
        })?;
        call.gas_price(gas_price)
    } else {
        call
    };
    call.send().await.map_err(ContractError::from)
}

pub async fn fetch_tx_input(
    provider: &NormalProvider,
    tx_hash: &str,
) -> ContractResult<Option<String>> {
    let hash =
        B256::from_str(tx_hash).map_err(|err| ContractError::transport("parsing tx hash", err))?;
    let tx = provider
        .get_transaction_by_hash(hash)
        .await
        .map_err(|err| ContractError::transport("fetching transaction by hash", err))?;
    Ok(tx.map(|t| format!("0x{}", hex::encode(t.input().as_ref()))))
}
