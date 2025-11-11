use std::{collections::VecDeque, sync::Arc};

use alloy::primitives::{Address, U256};
use api_types::indexer::{ProveManyRequest as RawProveManyRequest, TreeIndexResponse};
use async_trait::async_trait;
use reqwest::{Client, Url};
use thiserror::Error;
use tokio::sync::Mutex;

pub use api_types::indexer::{EventsQuery, HistoricalProof, IndexedEvent, TreeIndexQuery};

#[derive(Debug, Error)]
pub enum IndexerError {
    #[error("failed to build HTTP client for indexer")]
    ClientBuild(#[source] reqwest::Error),
    #[error("invalid indexer base url while joining path '{path}'")]
    InvalidEndpoint {
        path: String,
        #[source]
        source: url::ParseError,
    },
    #[error("failed to query indexer events endpoint")]
    EventsRequest(#[source] reqwest::Error),
    #[error("indexer events endpoint returned error status")]
    EventsStatus(#[source] reqwest::Error),
    #[error("failed to decode indexer events response")]
    EventsDecode(#[source] reqwest::Error),
    #[error("failed to submit indexer proof request")]
    ProofRequest(#[source] reqwest::Error),
    #[error("indexer proof endpoint returned error status")]
    ProofStatus(#[source] reqwest::Error),
    #[error("failed to decode indexer proof response")]
    ProofDecode(#[source] reqwest::Error),
    #[error("failed to query indexer tree index endpoint")]
    TreeIndexRequest(#[source] reqwest::Error),
    #[error("indexer tree index endpoint returned error status")]
    TreeIndexStatus(#[source] reqwest::Error),
    #[error("failed to decode indexer tree index response")]
    TreeIndexDecode(#[source] reqwest::Error),
    #[error("no queued response for {method} in TestIndexerClient")]
    TestQueueEmpty { method: &'static str },
}

pub type IndexerResult<T> = Result<T, IndexerError>;

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait IndexerClient: Send + Sync {
    async fn events_by_recipient(
        &self,
        chain_id: u64,
        token_address: Address,
        to: Address,
        limit: Option<usize>,
    ) -> IndexerResult<Vec<IndexedEvent>>;

    async fn prove_many(
        &self,
        chain_id: u64,
        token_address: Address,
        target_index: u64,
        leaf_indices: &[u64],
    ) -> IndexerResult<Vec<HistoricalProof>>;

    async fn tree_index_by_root(
        &self,
        chain_id: u64,
        token_address: Address,
        transfer_root: U256,
    ) -> IndexerResult<u64>;
}

#[derive(Clone, Debug)]
pub struct HttpIndexerClient {
    client: Client,
    base_url: Url,
}

impl HttpIndexerClient {
    pub fn new(base_url: Url) -> IndexerResult<Self> {
        let mut normalized = base_url.clone();
        if !normalized.path().ends_with('/') {
            let mut path = normalized.path().trim_end_matches('/').to_owned();
            path.push('/');
            normalized.set_path(&path);
        }

        let client = Client::builder()
            .build()
            .map_err(IndexerError::ClientBuild)?;

        Ok(Self {
            client,
            base_url: normalized,
        })
    }

    fn endpoint(&self, path: &str) -> IndexerResult<Url> {
        self.base_url
            .join(path)
            .map_err(|source| IndexerError::InvalidEndpoint {
                path: path.to_string(),
                source,
            })
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl IndexerClient for HttpIndexerClient {
    async fn events_by_recipient(
        &self,
        chain_id: u64,
        token_address: Address,
        to: Address,
        limit: Option<usize>,
    ) -> IndexerResult<Vec<IndexedEvent>> {
        let url = self.endpoint("events")?;
        let params = EventsQuery {
            chain_id,
            token_address,
            to,
            limit,
        };

        let response = self
            .client
            .get(url)
            .query(&params)
            .send()
            .await
            .map_err(IndexerError::EventsRequest)?
            .error_for_status()
            .map_err(IndexerError::EventsStatus)?;

        let events: Vec<IndexedEvent> =
            response.json().await.map_err(IndexerError::EventsDecode)?;

        Ok(events)
    }

    async fn prove_many(
        &self,
        chain_id: u64,
        token_address: Address,
        target_index: u64,
        leaf_indices: &[u64],
    ) -> IndexerResult<Vec<HistoricalProof>> {
        let url = self.endpoint("proofs")?;
        let payload = RawProveManyRequest {
            chain_id,
            token_address,
            target_index,
            leaf_indices: leaf_indices.to_vec(),
        };

        let response = self
            .client
            .post(url)
            .json(&payload)
            .send()
            .await
            .map_err(IndexerError::ProofRequest)?
            .error_for_status()
            .map_err(IndexerError::ProofStatus)?;

        let proofs: Vec<HistoricalProof> =
            response.json().await.map_err(IndexerError::ProofDecode)?;

        Ok(proofs)
    }

    async fn tree_index_by_root(
        &self,
        chain_id: u64,
        token_address: Address,
        transfer_root: U256,
    ) -> IndexerResult<u64> {
        let url = self.endpoint("tree-index")?;
        let params = TreeIndexQuery {
            chain_id,
            token_address,
            transfer_root,
        };

        let response = self
            .client
            .get(url)
            .query(&params)
            .send()
            .await
            .map_err(IndexerError::TreeIndexRequest)?
            .error_for_status()
            .map_err(IndexerError::TreeIndexStatus)?;

        let body: TreeIndexResponse = response
            .json()
            .await
            .map_err(IndexerError::TreeIndexDecode)?;
        Ok(body.tree_index)
    }
}

#[derive(Clone, Debug, Default)]
pub struct TestIndexerClient {
    events: Arc<Mutex<VecDeque<IndexerResult<Vec<IndexedEvent>>>>>,
    prove_many: Arc<Mutex<VecDeque<IndexerResult<Vec<HistoricalProof>>>>>,
    tree_index: Arc<Mutex<VecDeque<IndexerResult<u64>>>>,
}

impl TestIndexerClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn enqueue_events_response(&self, response: IndexerResult<Vec<IndexedEvent>>) {
        self.events.lock().await.push_back(response);
    }

    pub async fn enqueue_prove_many_response(&self, response: IndexerResult<Vec<HistoricalProof>>) {
        self.prove_many.lock().await.push_back(response);
    }

    pub async fn enqueue_tree_index_response(&self, response: IndexerResult<u64>) {
        self.tree_index.lock().await.push_back(response);
    }

    async fn take_next<T>(
        queue: &Arc<Mutex<VecDeque<IndexerResult<T>>>>,
        method: &'static str,
    ) -> IndexerResult<T> {
        queue
            .lock()
            .await
            .pop_front()
            .unwrap_or_else(|| Err(IndexerError::TestQueueEmpty { method }))
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl IndexerClient for TestIndexerClient {
    async fn events_by_recipient(
        &self,
        chain_id: u64,
        token_address: Address,
        to: Address,
        limit: Option<usize>,
    ) -> IndexerResult<Vec<IndexedEvent>> {
        let _ = (chain_id, token_address, to, limit);
        Self::take_next(&self.events, "events_by_recipient").await
    }

    async fn prove_many(
        &self,
        chain_id: u64,
        token_address: Address,
        target_index: u64,
        leaf_indices: &[u64],
    ) -> IndexerResult<Vec<HistoricalProof>> {
        let _ = (chain_id, token_address, target_index, leaf_indices);
        Self::take_next(&self.prove_many, "prove_many").await
    }

    async fn tree_index_by_root(
        &self,
        chain_id: u64,
        token_address: Address,
        transfer_root: U256,
    ) -> IndexerResult<u64> {
        let _ = (chain_id, token_address, transfer_root);
        Self::take_next(&self.tree_index, "tree_index_by_root").await
    }
}
