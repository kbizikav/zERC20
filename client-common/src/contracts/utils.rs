use alloy::{
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
use reqwest::Url;
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

pub fn get_provider(rpc_url: &str) -> anyhow::Result<NormalProvider> {
    let retry_layer = RetryBackoffLayer::new(5, 1000, 100);
    let url: Url = rpc_url
        .parse()
        .context(format!("Failed to parse rpc url: {}", rpc_url))?;
    let client = RpcClient::builder().layer(retry_layer).http(url);
    let provider = ProviderBuilder::default()
        .with_gas_estimation()
        .with_simple_nonce_management()
        .fetch_chain_id()
        .connect_client(client);
    Ok(provider)
}

pub fn get_provider_with_fallback(rpc_urls: &[String]) -> anyhow::Result<NormalProvider> {
    let retry_layer = RetryBackoffLayer::new(5, 1000, 100);
    let transports = rpc_urls
        .iter()
        .map(|url| {
            let url: Url = url
                .parse()
                .context(format!("Failed to parse rpc url: {}", url))?;
            Ok(Http::new(url))
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
        let gas_price = provider
            .get_gas_price()
            .await
            .map_err(|err| {
                ContractError::transport("fetching gas price for legacy transfer", err)
            })?;
        call.gas_price(gas_price)
    } else {
        call
    };
    call.send().await.map_err(ContractError::from)
}
