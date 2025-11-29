use alloy::primitives::Address;
use async_trait::async_trait;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LayerZeroError {
    #[error("failed to build HTTP client for LayerZero Scan")]
    ClientBuild(#[source] reqwest::Error),
    #[error("invalid LayerZero Scan base url while joining path '{path}'")]
    InvalidEndpoint {
        path: String,
        #[source]
        source: url::ParseError,
    },
    #[error("failed to query LayerZero Scan wallet messages")]
    WalletMessagesRequest(#[source] reqwest::Error),
    #[error("LayerZero Scan wallet messages returned {status}: {body}")]
    WalletMessagesStatus { status: StatusCode, body: String },
    #[error("failed to decode LayerZero Scan wallet messages response")]
    WalletMessagesDecode(#[source] reqwest::Error),
    #[error("failed to query LayerZero Scan transaction messages")]
    TxMessagesRequest(#[source] reqwest::Error),
    #[error("LayerZero Scan transaction messages returned {status}: {body}")]
    TxMessagesStatus { status: StatusCode, body: String },
    #[error("failed to decode LayerZero Scan transaction messages response")]
    TxMessagesDecode(#[source] reqwest::Error),
}

pub type LayerZeroResult<T> = Result<T, LayerZeroError>;

#[derive(Clone, Debug)]
pub struct HttpLayerZeroClient {
    client: Client,
    base_url: Url,
    api_key: Option<String>,
}

impl HttpLayerZeroClient {
    pub fn new(base_url: Url, api_key: Option<String>) -> LayerZeroResult<Self> {
        let mut normalized = base_url.clone();
        if !normalized.path().ends_with('/') {
            let mut path = normalized.path().trim_end_matches('/').to_owned();
            path.push('/');
            normalized.set_path(&path);
        }

        let client = Client::builder()
            .user_agent("curl/8.0 (zerc20-cli layerzero client)")
            .build()
            .map_err(LayerZeroError::ClientBuild)?;

        Ok(Self {
            client,
            base_url: normalized,
            api_key,
        })
    }

    fn endpoint(&self, path: &str) -> LayerZeroResult<Url> {
        self.base_url
            .join(path)
            .map_err(|source| LayerZeroError::InvalidEndpoint {
                path: path.to_string(),
                source,
            })
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait LayerZeroClient: Send + Sync {
    async fn wallet_messages(
        &self,
        src_address: Address,
        params: &WalletMessagesParams,
    ) -> LayerZeroResult<WalletMessagesResponse>;

    async fn tx_messages(&self, tx_hash: &str) -> LayerZeroResult<Option<TxMessagesResponse>>;
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl LayerZeroClient for HttpLayerZeroClient {
    async fn wallet_messages(
        &self,
        src_address: Address,
        params: &WalletMessagesParams,
    ) -> LayerZeroResult<WalletMessagesResponse> {
        let path = format!("messages/wallet/{:#x}", src_address);
        let url = self.endpoint(&path)?;

        let mut request = self.client.get(url).query(params);
        if let Some(api_key) = &self.api_key {
            request = request.header("x-api-key", api_key);
        }

        let response = request
            .send()
            .await
            .map_err(LayerZeroError::WalletMessagesRequest)?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read body>".to_string());
            return Err(LayerZeroError::WalletMessagesStatus { status, body });
        }

        let payload = response
            .json::<WalletMessagesResponse>()
            .await
            .map_err(LayerZeroError::WalletMessagesDecode)?;

        Ok(payload)
    }

    async fn tx_messages(&self, tx_hash: &str) -> LayerZeroResult<Option<TxMessagesResponse>> {
        let path = format!("messages/tx/{tx_hash}");
        let url = self.endpoint(&path)?;

        let mut request = self.client.get(url);
        if let Some(api_key) = &self.api_key {
            request = request.header("x-api-key", api_key);
        }

        let response = request
            .send()
            .await
            .map_err(LayerZeroError::TxMessagesRequest)?;

        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read body>".to_string());
            return Err(LayerZeroError::TxMessagesStatus { status, body });
        }

        let payload = response
            .json::<TxMessagesResponse>()
            .await
            .map_err(LayerZeroError::TxMessagesDecode)?;

        Ok(Some(payload))
    }
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WalletMessagesParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

pub type WalletMessagesResponse = ScanMessagesResponse<ScanMessage>;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScanMessagesResponse<T> {
    pub data: Vec<T>,
    #[serde(default)]
    pub next_token: Option<String>,
}

pub type TxMessagesResponse = ScanMessagesResponse<ScanMessage>;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ScanMessage {
    #[serde(default)]
    pub status: Option<Status>,
    #[serde(default)]
    pub pathway: Option<Pathway>,
    #[serde(default)]
    pub source: Option<Stage>,
    #[serde(default)]
    pub destination: Option<Destination>,
    #[serde(default)]
    pub verification: Option<Verification>,
    #[serde(default)]
    pub sealer: Option<Sealer>,
    #[serde(default)]
    pub guid: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Status {
    pub name: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Pathway {
    pub src_eid: Option<u64>,
    pub dst_eid: Option<u64>,
    pub sender: Option<Endpoint>,
    pub receiver: Option<Endpoint>,
    pub id: Option<String>,
    pub nonce: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Endpoint {
    pub address: Option<String>,
    pub chain: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Stage {
    pub status: Option<String>,
    pub tx: Option<TxInfo>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TxInfo {
    pub tx_hash: Option<String>,
    pub block_hash: Option<String>,
    #[serde(deserialize_with = "deserialize_opt_u64", default)]
    pub block_number: Option<u64>,
    #[serde(deserialize_with = "deserialize_opt_u64", default)]
    pub block_timestamp: Option<u64>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub payload: Option<String>,
    #[serde(deserialize_with = "deserialize_opt_u64", default)]
    pub readiness_timestamp: Option<u64>,
    #[serde(default)]
    pub options: Option<TxOptions>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TxOptions {
    #[serde(default)]
    pub lz_receive: Option<LzReceiveOptions>,
    #[serde(default)]
    pub compose: Option<Vec<ComposeOption>>,
    #[serde(default)]
    pub ordered: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LzReceiveOptions {
    #[serde(deserialize_with = "deserialize_opt_u64", default)]
    pub gas: Option<u64>,
    #[serde(deserialize_with = "deserialize_opt_u64", default)]
    pub value: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ComposeOption {
    #[serde(deserialize_with = "deserialize_opt_u64", default)]
    pub index: Option<u64>,
    #[serde(deserialize_with = "deserialize_opt_u64", default)]
    pub gas: Option<u64>,
    #[serde(deserialize_with = "deserialize_opt_u64", default)]
    pub value: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Verification {
    #[serde(default)]
    pub dvn: Option<Dvn>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Dvn {
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Sealer {
    pub status: Option<String>,
    pub tx: Option<TxInfo>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Destination {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub tx: Option<DestinationTx>,
    #[serde(default, rename = "lzCompose")]
    pub lz_compose: Option<LzCompose>,
    #[serde(default, rename = "nativeDrop")]
    pub native_drop: Option<NativeDrop>,
    #[serde(default)]
    pub payload: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DestinationTx {
    pub tx_hash: Option<String>,
    pub block_hash: Option<String>,
    #[serde(deserialize_with = "deserialize_opt_u64", default)]
    pub block_number: Option<u64>,
    #[serde(deserialize_with = "deserialize_opt_u64", default)]
    pub block_timestamp: Option<u64>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LzCompose {
    #[serde(default)]
    pub txs: Vec<LzComposeTx>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, rename = "failedTx")]
    pub failed_tx: Vec<LzComposeFailedTx>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LzComposeTx {
    pub tx_hash: Option<String>,
    pub block_hash: Option<String>,
    #[serde(deserialize_with = "deserialize_opt_u64", default)]
    pub block_number: Option<u64>,
    #[serde(deserialize_with = "deserialize_opt_u64", default)]
    pub block_timestamp: Option<u64>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LzComposeFailedTx {
    pub tx_hash: Option<String>,
    #[serde(rename = "txError")]
    pub tx_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NativeDrop {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub tx: Option<DestinationTx>,
}

pub fn deserialize_opt_u64<'de, D>(deserializer: D) -> std::result::Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<Value> = Option::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(Value::Number(num)) => num
            .as_u64()
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom("failed to parse numeric value")),
        Some(Value::String(text)) => text
            .parse::<u64>()
            .map(Some)
            .map_err(|err| serde::de::Error::custom(err.to_string())),
        Some(other) => Err(serde::de::Error::custom(format!(
            "unexpected value type for u64: {}",
            other
        ))),
    }
}
