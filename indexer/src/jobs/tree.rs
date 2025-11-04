use std::{cmp::min, convert::TryFrom, time::Instant};

use alloy::primitives::{Address, U256};
use anyhow::{Context, Result, bail};
use log::{debug, error, info, warn};
use sqlx::{FromRow, PgPool};

use crate::{
    config::TreeJobConfig,
    trees::{DbIncrementalMerkleTree, DbMerkleTreeConfig},
};
use client_common::tokens::{TokenEntry, TokenMetadata};

use super::try_acquire_lock;

const TREE_LOCK_SALT: u64 = 0x54524545; // "TREE"

#[derive(Clone)]
pub struct TreeIngestionJob {
    pool: PgPool,
    tokens: Vec<TreeTokenContext>,
    interval_ms: u64,
    tree_height: u32,
    batch_size: usize,
    tree_config: DbMerkleTreeConfig,
}

impl TreeIngestionJob {
    pub async fn run_forever(&self) -> Result<()> {
        loop {
            let iteration_started = Instant::now();
            self.run_once().await;
            let elapsed = iteration_started.elapsed();
            let interval = std::time::Duration::from_millis(self.interval_ms);
            if elapsed < interval {
                tokio::time::sleep(interval - elapsed).await;
            }
        }
    }

    pub async fn run_once(&self) {
        for token in &self.tokens {
            if let Err(err) = self.process_token(token).await {
                error!("tree ingestion failed for token '{}': {err:?}", token.label);
            }
        }
    }

    async fn process_token(&self, token: &TreeTokenContext) -> Result<()> {
        let Some(lease) = try_acquire_lock(&self.pool, token.lock_key).await? else {
            debug!(
                "skip tree ingestion for '{}' due to lock contention",
                token.label
            );
            return Ok(());
        };

        let ingest_result = self.ingest_token(token).await;

        if let Err(err) = lease.release().await {
            warn!(
                "failed to release tree lease for '{}': {err:?}",
                token.label
            );
        }

        ingest_result
    }

    async fn ingest_token(&self, token: &TreeTokenContext) -> Result<()> {
        let chain_id_i64 = i64::try_from(token.metadata.chain_id)
            .context("chain_id exceeds i64 range for tree ingestion")?;
        let token_id: Option<i64> = sqlx::query_scalar(
            r#"
            SELECT id
            FROM tokens
            WHERE token_address = $1 AND chain_id = $2
            "#,
        )
        .bind(token.metadata.token_address.as_slice())
        .bind(chain_id_i64)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("failed to locate token '{}'", token.label))?;

        let Some(token_id) = token_id else {
            debug!(
                "token '{}' not yet registered in database; waiting for event sync",
                token.label
            );
            return Ok(());
        };

        let tree = DbIncrementalMerkleTree::new(
            self.pool.clone(),
            token_id,
            self.tree_height,
            self.tree_config.clone(),
        )
        .await
        .with_context(|| format!("failed to initialise merkle tree for '{}'", token.label))?;

        let processed = latest_tree_index(&self.pool, token_id).await?;
        let contiguous_index = contiguous_event_index(&self.pool, token_id).await?;
        let target_event_count = match contiguous_index {
            None => 0,
            Some(idx) => idx + 1,
        };

        if processed > target_event_count {
            warn!(
                "tree state ahead of events for '{}': processed={}, contiguous_events={}",
                token.label, processed, target_event_count
            );
            return Ok(());
        }

        if processed == target_event_count {
            debug!("no new events to ingest for '{}'", token.label);
            return Ok(());
        }

        ingest_events(
            &self.pool,
            &tree,
            token_id,
            processed,
            target_event_count,
            self.batch_size,
            &token.label,
        )
        .await?;

        info!(
            "tree ingestion completed for '{}' (processed {} events)",
            token.label,
            target_event_count - processed
        );

        Ok(())
    }
}

pub struct TreeIngestionJobBuilder {
    pool: PgPool,
    job_config: TreeJobConfig,
    tokens: Vec<TokenEntry>,
}

impl TreeIngestionJobBuilder {
    pub fn new(pool: PgPool, job_config: TreeJobConfig, tokens: Vec<TokenEntry>) -> Self {
        Self {
            pool,
            job_config,
            tokens,
        }
    }

    pub fn into_job(self) -> Result<TreeIngestionJob> {
        let tree_config = self
            .job_config
            .build_tree_config()
            .context("invalid tree configuration")?;

        let contexts = self
            .tokens
            .into_iter()
            .map(|token| {
                let metadata = token.metadata();
                TreeTokenContext {
                    label: token.label.clone(),
                    metadata,
                    lock_key: token.lock_key_with_salt(TREE_LOCK_SALT),
                }
            })
            .collect();

        Ok(TreeIngestionJob {
            pool: self.pool,
            tokens: contexts,
            interval_ms: self.job_config.interval_ms,
            tree_height: self.job_config.height,
            batch_size: self.job_config.batch_size,
            tree_config,
        })
    }
}

#[derive(Clone)]
struct TreeTokenContext {
    label: String,
    metadata: TokenMetadata,
    lock_key: i64,
}

#[derive(FromRow)]
struct EventRow {
    event_index: i64,
    to_address: Vec<u8>,
    value: Vec<u8>,
}

async fn ingest_events(
    pool: &PgPool,
    tree: &DbIncrementalMerkleTree,
    token_id: i64,
    mut processed: u64,
    target_event_count: u64,
    batch_size: usize,
    label: &str,
) -> Result<()> {
    loop {
        if processed >= target_event_count {
            break;
        }

        let remaining = target_event_count - processed;
        let batch = min(remaining, batch_size as u64);
        let start_index =
            i64::try_from(processed).context("processed index exceeds i64 for event ingestion")?;
        let upper_index = i64::try_from(target_event_count - 1)
            .context("target index exceeds i64 for event ingestion")?;
        let limit = i64::try_from(batch).unwrap_or(i64::MAX);

        let events: Vec<EventRow> = sqlx::query_as(
            r#"
            SELECT event_index, to_address, value
            FROM indexed_transfer_events
            WHERE token_id = $1
              AND event_index >= $2
              AND event_index <= $3
            ORDER BY event_index
            LIMIT $4
            "#,
        )
        .bind(token_id)
        .bind(start_index)
        .bind(upper_index)
        .bind(limit)
        .fetch_all(pool)
        .await
        .with_context(|| format!("failed to fetch events for token '{label}'"))?;

        if events.is_empty() {
            debug!(
                "no events fetched for '{}', processed={}, target={}",
                label, processed, target_event_count
            );
            break;
        }

        for event in events {
            let event_index_u64 = u64::try_from(event.event_index).with_context(|| {
                format!(
                    "event index negative for token '{label}': {}",
                    event.event_index
                )
            })?;

            if event_index_u64 < processed {
                debug!(
                    "skipping already processed event {} for '{}'",
                    event_index_u64, label
                );
                continue;
            }

            if event_index_u64 != processed {
                warn!(
                    "non contiguous event sequence for '{}': expected {}, saw {}",
                    label, processed, event_index_u64
                );
                return Ok(());
            }

            let to_address =
                parse_address(&event.to_address).context("invalid to_address bytes")?;
            let value = parse_u256(&event.value).context("invalid value bytes")?;

            let append = tree
                .append_leaf(to_address, value)
                .await
                .with_context(|| format!("failed to append leaf for token '{label}'"))?;

            let expected_index = processed + 1;
            if append.index != expected_index {
                warn!(
                    "tree index mismatch for '{}': expected {}, got {}",
                    label, expected_index, append.index
                );
            }
            processed = append.index;
        }
    }

    Ok(())
}

async fn latest_tree_index(pool: &PgPool, token_id: i64) -> Result<u64> {
    let latest: i64 = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COALESCE(MAX(tree_index), 0)
        FROM merkle_snapshots
        WHERE token_id = $1
        "#,
    )
    .bind(token_id)
    .fetch_one(pool)
    .await
    .context("failed to query latest tree index")?;
    Ok(latest.max(0) as u64)
}

async fn contiguous_event_index(pool: &PgPool, token_id: i64) -> Result<Option<u64>> {
    let row: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT contiguous_index
        FROM event_indexer_state
        WHERE token_id = $1
        "#,
    )
    .bind(token_id)
    .fetch_optional(pool)
    .await
    .context("failed to query contiguous event index")?;
    match row {
        Some(value) if value >= 0 => Ok(Some(value as u64)),
        _ => Ok(None),
    }
}

fn parse_address(bytes: &[u8]) -> Result<Address> {
    if bytes.len() != 20 {
        bail!("address bytes must be 20, got {}", bytes.len());
    }
    Ok(Address::from_slice(bytes))
}

fn parse_u256(bytes: &[u8]) -> Result<U256> {
    if bytes.len() != 32 {
        bail!("value bytes must be 32, got {}", bytes.len());
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(bytes);
    Ok(U256::from_be_bytes(arr))
}
