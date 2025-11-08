use std::{collections::HashMap, sync::Arc};

use actix_cors::Cors;
use actix_web::{
    App, HttpResponse, HttpServer, Responder,
    error::{ErrorBadRequest, ErrorInternalServerError, ErrorNotFound},
    web::{self, Data, Json, Query},
};
use alloy::primitives::{Address, U256};
use anyhow::{Context, Result, anyhow};
use api_types::indexer::{
    EventsQuery, HistoricalProof, IndexedEvent, ProveManyRequest, TokenStatusResponse,
    TreeIndexQuery, TreeIndexResponse,
};
use ark_bn254::Fr;
use ark_ff::{BigInteger, PrimeField};
use log::{error, warn};
use sqlx::{PgPool, Row};

use crate::trees::{DbIncrementalMerkleTree, DbMerkleTreeConfig, DbMerkleTreeError};
use client_common::{
    contracts::{utils::get_provider_with_fallback, verifier::VerifierContract},
    tokens::{TokenEntry, TokenMetadata},
};

#[derive(Clone)]
pub struct AppState {
    pool: PgPool,
    tokens: Arc<TokenRegistry>,
    tree_config: DbMerkleTreeConfig,
    tree_height: u32,
}

impl AppState {
    fn new(
        pool: PgPool,
        tokens: TokenRegistry,
        tree_config: DbMerkleTreeConfig,
        tree_height: u32,
    ) -> Self {
        Self {
            pool,
            tokens: Arc::new(tokens),
            tree_config,
            tree_height,
        }
    }

    fn token(&self, chain_id: u64, token_address: &Address) -> Option<&TokenContext> {
        self.tokens.get(chain_id, token_address)
    }

    fn token_contexts(&self) -> Vec<TokenContext> {
        self.tokens.all()
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct TokenKey {
    chain_id: u64,
    address: Address,
}

#[derive(Clone, Debug)]
struct TokenContext {
    id: i64,
    label: String,
    chain_id: u64,
    token_address: Address,
    verifier_address: Address,
    rpc_urls: Vec<String>,
    legacy_tx: bool,
}

#[derive(Clone, Debug)]
struct TokenRegistry {
    by_key: HashMap<TokenKey, TokenContext>,
}

impl TokenRegistry {
    pub async fn initialise(pool: &PgPool, tokens: &[TokenEntry]) -> Result<Self> {
        let mut registry = HashMap::with_capacity(tokens.len());
        for token in tokens {
            let metadata = token.metadata();
            let token_id = ensure_token_record(pool, &metadata)
                .await
                .with_context(|| format!("failed to ensure token '{}'", token.label))?;
            let key = TokenKey {
                chain_id: metadata.chain_id,
                address: metadata.token_address,
            };
            registry.insert(
                key,
                TokenContext {
                    id: token_id,
                    label: token.label.clone(),
                    chain_id: metadata.chain_id,
                    token_address: metadata.token_address,
                    verifier_address: metadata.verifier_address,
                    rpc_urls: token.rpc_urls.clone(),
                    legacy_tx: token.legacy_tx,
                },
            );
        }
        Ok(Self { by_key: registry })
    }

    fn get(&self, chain_id: u64, token_address: &Address) -> Option<&TokenContext> {
        let key = TokenKey {
            chain_id,
            address: *token_address,
        };
        self.by_key.get(&key)
    }

    fn all(&self) -> Vec<TokenContext> {
        self.by_key.values().cloned().collect()
    }
}

pub async fn run_http_server(
    bind_addr: &str,
    pool: PgPool,
    tokens: &[TokenEntry],
    tree_config: DbMerkleTreeConfig,
    tree_height: u32,
) -> Result<()> {
    let registry = TokenRegistry::initialise(&pool, tokens)
        .await
        .context("initialise token registry")?;
    let state = AppState::new(pool, registry, tree_config, tree_height);
    let shared_state = Data::new(state);

    HttpServer::new(move || {
        App::new()
            .wrap(Cors::permissive())
            .app_data(shared_state.clone())
            .route("/healthz", web::get().to(health))
            .route("/status", web::get().to(tokens_status))
            .route("/events", web::get().to(events_by_recipient))
            .route("/proofs", web::post().to(prove_many))
            .route("/tree-index", web::get().to(tree_index_by_root))
    })
    .bind(bind_addr)
    .with_context(|| format!("failed to bind HTTP server to {bind_addr}"))?
    .run()
    .await
    .context("HTTP server terminated unexpectedly")?;

    Ok(())
}

async fn health() -> impl Responder {
    HttpResponse::Ok().finish()
}

async fn tokens_status(state: Data<AppState>) -> actix_web::Result<Json<Vec<TokenStatusResponse>>> {
    let contexts = state.token_contexts();
    let mut statuses = Vec::with_capacity(contexts.len());

    for token in contexts {
        let (reserved_index, proved_index) = fetch_onchain_indices(&token).await;

        let events_synced_index = fetch_events_synced_index(&state.pool, token.id)
            .await
            .map_err(|err| {
                error!(
                    "failed to load event index for token '{}': {err:?}",
                    token.label
                );
                ErrorInternalServerError("failed to load event index")
            })?;

        let tree_synced_index = fetch_tree_synced_index(&state.pool, token.id)
            .await
            .map_err(|err| {
                error!(
                    "failed to load tree index for token '{}': {err:?}",
                    token.label
                );
                ErrorInternalServerError("failed to load tree index")
            })?;

        let ivc_generated_index = fetch_ivc_generated_index(&state.pool, token.id)
            .await
            .map_err(|err| {
                error!(
                    "failed to load ivc index for token '{}': {err:?}",
                    token.label
                );
                ErrorInternalServerError("failed to load ivc index")
            })?;

        statuses.push(TokenStatusResponse {
            label: token.label.clone(),
            chain_id: token.chain_id,
            token_address: token.token_address,
            verifier_address: token.verifier_address,
            onchain_reserved_index: reserved_index,
            onchain_proved_index: proved_index,
            events_synced_index,
            tree_synced_index,
            ivc_generated_index,
        });
    }

    Ok(Json(statuses))
}

async fn events_by_recipient(
    state: Data<AppState>,
    query: Query<EventsQuery>,
) -> actix_web::Result<Json<Vec<IndexedEvent>>> {
    let params = query.into_inner();
    let token = state
        .token(params.chain_id, &params.token_address)
        .ok_or_else(|| {
            ErrorNotFound(format!(
                "token not configured for chain_id {} and address {}",
                params.chain_id, params.token_address
            ))
        })?;

    let to_address = params.to;

    let limit = params.limit.unwrap_or(100);
    let limit = limit.min(1_000);
    let limit_i64 = i64::try_from(limit).map_err(|_| ErrorBadRequest("limit is too large"))?;

    let rows = sqlx::query(
        r#"
        SELECT event_index, from_address, to_address, value, eth_block_number
        FROM indexed_transfer_events
        WHERE token_id = $1
          AND to_address = $2
        ORDER BY event_index ASC
        LIMIT $3
        "#,
    )
    .bind(token.id)
    .bind(to_address.as_slice())
    .bind(limit_i64)
    .fetch_all(&state.pool)
    .await
    .map_err(|err| {
        error!(
            "failed to fetch events for token '{}' and address {}: {err:?}",
            token.label, params.to
        );
        ErrorInternalServerError("failed to fetch events")
    })?;

    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let event_index: i64 = row.try_get("event_index").map_err(|_| {
            ErrorInternalServerError("invalid event_index value retrieved from database")
        })?;
        let from_bytes: Vec<u8> = row.try_get("from_address").map_err(|_| {
            ErrorInternalServerError("invalid from_address value retrieved from database")
        })?;
        let to_bytes: Vec<u8> = row.try_get("to_address").map_err(|_| {
            ErrorInternalServerError("invalid to_address value retrieved from database")
        })?;
        let value_bytes: Vec<u8> = row
            .try_get("value")
            .map_err(|_| ErrorInternalServerError("invalid value retrieved from database"))?;
        let block_number: i64 = row.try_get("eth_block_number").map_err(|_| {
            ErrorInternalServerError("invalid eth_block_number retrieved from database")
        })?;

        let event_index = u64::try_from(event_index)
            .map_err(|_| ErrorInternalServerError("event_index does not fit into u64"))?;
        let block_number = u64::try_from(block_number)
            .map_err(|_| ErrorInternalServerError("block number does not fit into u64"))?;

        let from = address_from_bytes(&from_bytes)?;
        let to = address_from_bytes(&to_bytes)?;
        let value = bytes32_to_u256(&value_bytes)
            .map_err(|_| ErrorInternalServerError("stored value must be 32 bytes"))?;

        events.push(IndexedEvent {
            event_index,
            from,
            to,
            value,
            eth_block_number: block_number,
        });
    }

    Ok(Json(events))
}

async fn prove_many(
    state: Data<AppState>,
    request: Json<ProveManyRequest>,
) -> actix_web::Result<Json<Vec<HistoricalProof>>> {
    let request = request.into_inner();
    let token = state
        .token(request.chain_id, &request.token_address)
        .ok_or_else(|| {
            ErrorNotFound(format!(
                "token not configured for chain_id {} and address {:#x}",
                request.chain_id, request.token_address
            ))
        })?;

    if request.leaf_indices.is_empty() {
        return Ok(Json(Vec::new()));
    }

    let tree = DbIncrementalMerkleTree::new(
        state.pool.clone(),
        token.id,
        state.tree_height,
        state.tree_config.clone(),
    )
    .await
    .map_err(map_merkle_error)?;

    let proofs = tree
        .prove_many(request.target_index, &request.leaf_indices)
        .await
        .map_err(map_merkle_error)?;

    let responses = proofs
        .into_iter()
        .map(|proof| HistoricalProof {
            target_index: proof.target_index,
            leaf_index: proof.leaf_index,
            root: fr_to_u256(proof.root),
            hash_chain: proof.hash_chain,
            siblings: proof.proof.siblings.into_iter().map(fr_to_u256).collect(),
        })
        .collect();

    Ok(Json(responses))
}

async fn tree_index_by_root(
    state: Data<AppState>,
    query: Query<TreeIndexQuery>,
) -> actix_web::Result<Json<TreeIndexResponse>> {
    let params = query.into_inner();
    let token = state
        .token(params.chain_id, &params.token_address)
        .ok_or_else(|| {
            ErrorNotFound(format!(
                "token not configured for chain_id {} and address {}",
                params.chain_id, params.token_address
            ))
        })?;

    let root_bytes = params.transfer_root.to_be_bytes::<32>();

    let row = sqlx::query(
        r#"
        SELECT tree_index
        FROM merkle_snapshots
        WHERE token_id = $1
          AND root_hash = $2
        ORDER BY tree_index DESC
        LIMIT 1
        "#,
    )
    .bind(token.id)
    .bind(root_bytes.as_slice())
    .fetch_optional(&state.pool)
    .await
    .map_err(|err| {
        error!(
            "failed to lookup tree index for token '{}' and root {}: {err:?}",
            token.label,
            format!("{:#x}", params.transfer_root)
        );
        ErrorInternalServerError("failed to lookup transfer root")
    })?;

    let Some(row) = row else {
        return Err(ErrorNotFound("transfer root not found"));
    };

    let index: i64 = row
        .try_get("tree_index")
        .map_err(|_| ErrorInternalServerError("invalid tree_index value stored in database"))?;
    let tree_index = u64::try_from(index)
        .map_err(|_| ErrorInternalServerError("tree_index does not fit into u64"))?;

    Ok(Json(TreeIndexResponse { tree_index }))
}

async fn ensure_token_record(pool: &PgPool, metadata: &TokenMetadata) -> Result<i64> {
    let chain_id = i64::try_from(metadata.chain_id)
        .map_err(|_| anyhow!("chain_id {} exceeds i64", metadata.chain_id))?;

    let sql = r#"
        INSERT INTO tokens (token_address, verifier_address, chain_id)
        VALUES ($1, $2, $3)
        ON CONFLICT (token_address, chain_id)
        DO UPDATE
        SET verifier_address = EXCLUDED.verifier_address,
            updated_at = NOW()
        RETURNING id
    "#;

    let id = sqlx::query_scalar::<_, i64>(sql)
        .bind(metadata.token_address.as_slice())
        .bind(metadata.verifier_address.as_slice())
        .bind(chain_id)
        .fetch_one(pool)
        .await
        .context("failed to upsert token record")?;

    Ok(id)
}

fn address_from_bytes(bytes: &[u8]) -> actix_web::Result<Address> {
    if bytes.len() != 20 {
        return Err(ErrorInternalServerError("address must be 20 bytes"));
    }
    let mut arr = [0u8; 20];
    arr.copy_from_slice(bytes);
    Ok(Address::from(arr))
}

fn bytes32_to_u256(bytes: &[u8]) -> Result<U256, ()> {
    if bytes.len() != 32 {
        return Err(());
    }
    Ok(U256::from_be_slice(bytes))
}

fn fr_to_u256(value: Fr) -> U256 {
    let bigint = value.into_bigint();
    let mut bytes = bigint.to_bytes_be();
    if bytes.len() < 32 {
        let mut padded = vec![0u8; 32 - bytes.len()];
        padded.extend_from_slice(&bytes);
        bytes = padded;
    }
    if bytes.len() > 32 {
        bytes = bytes[bytes.len() - 32..].to_vec();
    }
    U256::from_be_slice(&bytes)
}

async fn fetch_onchain_indices(token: &TokenContext) -> (Option<u64>, Option<u64>) {
    if token.rpc_urls.is_empty() {
        warn!(
            "token '{}' has no rpc_urls configured; skipping on-chain index lookup",
            token.label
        );
        return (None, None);
    }

    let provider = match get_provider_with_fallback(&token.rpc_urls) {
        Ok(provider) => provider,
        Err(err) => {
            error!(
                "failed to build provider for token '{}': {err:?}",
                token.label
            );
            return (None, None);
        }
    };

    let contract =
        VerifierContract::new(provider, token.verifier_address).with_legacy_tx(token.legacy_tx);

    let reserved_index = match contract.latest_reserved_index().await {
        Ok(value) => Some(value),
        Err(err) => {
            warn!(
                "failed to fetch latest_reserved_index for token '{}': {err:?}",
                token.label
            );
            None
        }
    };

    let proved_index = match contract.latest_proved_index().await {
        Ok(value) => Some(value),
        Err(err) => {
            warn!(
                "failed to fetch latest_proved_index for token '{}': {err:?}",
                token.label
            );
            None
        }
    };

    (reserved_index, proved_index)
}

async fn fetch_events_synced_index(
    pool: &PgPool,
    token_id: i64,
) -> Result<Option<u64>, sqlx::Error> {
    let value: Option<Option<i64>> = sqlx::query_scalar::<_, Option<i64>>(
        r#"
        SELECT contiguous_index
        FROM event_indexer_state
        WHERE token_id = $1
        "#,
    )
    .bind(token_id)
    .fetch_optional(pool)
    .await?;

    Ok(value
        .flatten()
        .and_then(|v| if v >= 0 { Some(v as u64) } else { None }))
}

async fn fetch_tree_synced_index(pool: &PgPool, token_id: i64) -> Result<Option<u64>, sqlx::Error> {
    let value: Option<Option<i64>> = sqlx::query_scalar::<_, Option<i64>>(
        r#"
        SELECT MAX(tree_index)
        FROM merkle_snapshots
        WHERE token_id = $1
        "#,
    )
    .bind(token_id)
    .fetch_optional(pool)
    .await?;

    Ok(value.flatten().map(|v| v.max(0) as u64))
}

async fn fetch_ivc_generated_index(
    pool: &PgPool,
    token_id: i64,
) -> Result<Option<u64>, sqlx::Error> {
    let value: Option<Option<i64>> = sqlx::query_scalar::<_, Option<i64>>(
        r#"
        SELECT MAX(end_index)
        FROM root_ivc_proofs
        WHERE token_id = $1
        "#,
    )
    .bind(token_id)
    .fetch_optional(pool)
    .await?;

    Ok(value.flatten().map(|v| v.max(0) as u64))
}

fn map_merkle_error(err: DbMerkleTreeError) -> actix_web::Error {
    match err {
        DbMerkleTreeError::Database { .. } => {
            ErrorInternalServerError("merkle tree database error")
        }
        DbMerkleTreeError::TokenNotFound { .. }
        | DbMerkleTreeError::MissingRoot { .. }
        | DbMerkleTreeError::MissingHashChain { .. }
        | DbMerkleTreeError::TreeEmpty => ErrorNotFound(err.to_string()),
        DbMerkleTreeError::InvalidTokenId { .. }
        | DbMerkleTreeError::InvalidHeight { .. }
        | DbMerkleTreeError::LeafIndexOverflow
        | DbMerkleTreeError::InvalidProofTargetZero
        | DbMerkleTreeError::TargetIndexTooHigh { .. }
        | DbMerkleTreeError::RetentionWindowExceeded { .. }
        | DbMerkleTreeError::LeafIndexOutOfBounds { .. }
        | DbMerkleTreeError::InvalidFrBytes { .. }
        | DbMerkleTreeError::InvalidU256Bytes { .. }
        | DbMerkleTreeError::InvalidBitPathBytes { .. }
        | DbMerkleTreeError::InvalidConfig { .. } => ErrorBadRequest(err.to_string()),
        DbMerkleTreeError::U64ToI64 { .. } | DbMerkleTreeError::I64ToU64 { .. } => {
            ErrorInternalServerError("integer conversion error during merkle operation")
        }
    }
}
