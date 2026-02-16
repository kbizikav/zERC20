use std::{convert::TryFrom, num::NonZeroU64};

use alloy::{
    eips::BlockNumberOrTag,
    primitives::{B256, U256},
};
use api_types::indexer::IndexedEvent;
use serde::Deserialize;
use sqlx::{FromRow, PgPool, Postgres, QueryBuilder, Transaction};
use thiserror::Error;

use client_common::{
    contracts::{ContractError, z_erc20::ZErc20Contract},
    tokens::TokenMetadata,
};
use log::warn;

pub const BLOCK_SPAN_RECOMMENDED: u64 = 5_000;
pub const FORWARD_SCAN_OVERLAP_RECOMMENDED: u64 = 10;
pub const REORG_CHECK_WINDOW_RECOMMENDED: u64 = 5_000;
const VALUE_BYTES: usize = 32;
const EVENTS_TABLE: &str = "indexed_transfer_events";
const BLOCKS_TABLE: &str = "indexed_blocks";
const STATE_TABLE: &str = "event_indexer_state";
const TOKENS_TABLE: &str = "tokens";
const ADVANCE_BATCH_SIZE: i64 = 512;

pub type Result<T> = std::result::Result<T, EventIndexerError>;

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BlockTag {
    #[default]
    Latest,
    Safe,
    Finalized,
}

#[derive(Debug, Error)]
pub enum EventIndexerError {
    #[error("invalid token id {token_id} for partitioning")]
    InvalidTokenId { token_id: i64 },
    #[error("{label} configuration value must be greater than zero")]
    NonPositiveConfig { label: &'static str },
    #[error("{label} negative or overflow: {value}")]
    I64ToU64 { label: &'static str, value: i64 },
    #[error("{label} exceeds i64: {value}")]
    U64ToI64 { label: &'static str, value: u64 },
    #[error("database error while {action}")]
    Database {
        action: &'static str,
        #[source]
        source: sqlx::Error,
    },
    #[error("contract error during {action}")]
    Contract {
        action: &'static str,
        #[source]
        source: ContractError,
    },
    #[error("failed inserting event index {index}")]
    InsertEvent {
        index: u64,
        #[source]
        source: sqlx::Error,
    },
}

impl EventIndexerError {
    fn database(action: &'static str, source: sqlx::Error) -> Self {
        Self::Database { action, source }
    }

    fn contract(action: &'static str, source: ContractError) -> Self {
        Self::Contract { action, source }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EventIndexerConfig {
    block_span: NonZeroU64,
    forward_scan_overlap: u64,
    block_tag: BlockTag,
    reorg_check_window: u64,
}

impl EventIndexerConfig {
    pub fn new(
        block_span: u64,
        forward_scan_overlap: u64,
        block_tag: BlockTag,
        reorg_check_window: u64,
    ) -> Result<Self> {
        let Some(block_span) = NonZeroU64::new(block_span) else {
            return Err(EventIndexerError::NonPositiveConfig {
                label: "block_span",
            });
        };
        Ok(Self {
            block_span,
            forward_scan_overlap,
            block_tag,
            reorg_check_window,
        })
    }

    pub fn block_span(&self) -> NonZeroU64 {
        self.block_span
    }

    pub fn forward_scan_overlap(&self) -> u64 {
        self.forward_scan_overlap
    }

    pub fn block_tag(&self) -> BlockTag {
        self.block_tag
    }

    pub fn reorg_check_window(&self) -> u64 {
        self.reorg_check_window
    }
}

impl From<BlockTag> for BlockNumberOrTag {
    fn from(tag: BlockTag) -> Self {
        match tag {
            BlockTag::Latest => BlockNumberOrTag::Latest,
            BlockTag::Safe => BlockNumberOrTag::Safe,
            BlockTag::Finalized => BlockNumberOrTag::Finalized,
        }
    }
}

pub struct EventIndexer {
    label: String,
    contract: ZErc20Contract,
    pool: PgPool,
    deployed_block_number: u64,
    partitions: EventIndexerPartitions,
    config: EventIndexerConfig,
}

impl EventIndexer {
    pub async fn new(
        contract: ZErc20Contract,
        pool: PgPool,
        deployed_block_number: u64,
        metadata: TokenMetadata,
        config: EventIndexerConfig,
        label: impl Into<String>,
    ) -> Result<Self> {
        let token_id = ensure_token_record(&pool, &metadata).await?;
        let partitions = EventIndexerPartitions::new(token_id)?;
        Ok(Self {
            label: label.into(),
            contract,
            pool,
            deployed_block_number,
            partitions,
            config,
        })
    }

    pub async fn sync(&self) -> Result<()> {
        self.partitions.ensure(&self.pool).await?;

        let mut state = ensure_state_row(
            &self.pool,
            self.partitions.token_id(),
            self.deployed_block_number,
        )
        .await?;

        if let Some(reorg_block) = self.detect_reorg(&state).await? {
            warn!(
                "reorg detected for '{}' at block {}; rolling back",
                self.label, reorg_block
            );
            self.rollback_to_block(reorg_block).await?;
            state = ensure_state_row(
                &self.pool,
                self.partitions.token_id(),
                self.deployed_block_number,
            )
            .await?;
        }

        let latest_block = self
            .contract
            .latest_block_by_tag(self.config.block_tag().into())
            .await
            .map_err(|err| EventIndexerError::contract("latest_block_by_tag", err))?;
        let contract_next_index = self
            .contract
            .index()
            .await
            .map_err(|err| EventIndexerError::contract("index", err))?;
        let expected_last_index = contract_next_index.checked_sub(1);

        let forward_start = state
            .last_synced_block
            .saturating_sub(self.config.forward_scan_overlap())
            .max(self.deployed_block_number);

        if forward_start <= latest_block {
            self.scan_chunked(forward_start, latest_block).await?;
        }

        persist_sync_watermark(
            &self.pool,
            self.partitions.token_id(),
            latest_block,
            contract_next_index,
        )
        .await?;

        state = advance_contiguous_index(&self.pool, self.partitions.token_id()).await?;

        let _ = self
            .backfill_missing_indices(state, expected_last_index, latest_block)
            .await?;

        prune_old_blocks(
            &self.pool,
            self.partitions.token_id(),
            latest_block,
            self.config.reorg_check_window(),
        )
        .await?;

        Ok(())
    }

    async fn scan_chunked(&self, from_block: u64, to_block: u64) -> Result<()> {
        if from_block > to_block {
            return Ok(());
        }

        let max_block_span = self.config.block_span().get();
        let forward_overlap = self.config.forward_scan_overlap();
        let mut from = from_block;
        let mut current_span = max_block_span;

        while from <= to_block {
            let to = to_block.min(from.saturating_add(current_span - 1));
            let fetched = match self.contract.get_indexed_transfer_events(from, to).await {
                Ok(events) => events,
                Err(err) => {
                    if is_invalid_block_range_error(&err) && current_span > 1 {
                        let previous_span = current_span;
                        current_span = (current_span / 2).max(1);
                        warn!(
                            "provider rejected block range [{from}, {to}] for '{}' (contract {}); reducing span from {} to {}",
                            self.label,
                            self.contract.address(),
                            previous_span,
                            current_span,
                        );
                        continue;
                    }

                    // Log a hint if this might be an unrecognized block range error pattern.
                    // This helps operators identify new patterns that should be added.
                    if current_span > 1 && is_potential_unrecognized_block_range_error(&err) {
                        warn!(
                            "unrecognized error during block range query [{from}, {to}] for '{}' - \
                            this may be a new block range limit pattern that should be added to \
                            is_invalid_block_range_error(): {}",
                            self.label, err
                        );
                    }

                    return Err(EventIndexerError::contract(
                        "get_indexed_transfer_events",
                        err,
                    ));
                }
            };

            if !fetched.is_empty() {
                let blocks = self.fetch_block_hashes(&fetched).await?;
                if !blocks.is_empty() {
                    upsert_indexed_blocks(&self.pool, self.partitions.token_id(), &blocks).await?;
                }
                insert_events(&self.pool, self.partitions.token_id(), &fetched).await?;
            }

            if to == to_block {
                break;
            }

            if current_span < max_block_span {
                current_span = current_span.saturating_mul(2).min(max_block_span);
            }

            let next_from = to.saturating_add(1);
            // Cap overlap to current_span - 1 so `from` always advances by at least 1.
            // Without this, when current_span < forward_overlap the subtraction
            // pushes `from` backward, creating an infinite loop.
            let effective_overlap = forward_overlap.min(current_span.saturating_sub(1));
            from = next_from.saturating_sub(effective_overlap.min(next_from));
        }

        Ok(())
    }

    async fn backfill_missing_indices(
        &self,
        mut state: IndexerState,
        expected_last_index: Option<u64>,
        latest_block: u64,
    ) -> Result<IndexerState> {
        let Some(target_last_index) = expected_last_index else {
            return Ok(state);
        };

        loop {
            if state.contiguous_index >= 0 && state.contiguous_index as u64 >= target_last_index {
                break;
            }

            let Some(anchor) = find_gap_anchor(
                &self.pool,
                self.partitions.token_id(),
                &state,
                target_last_index,
                latest_block,
                self.deployed_block_number,
            )
            .await?
            else {
                break;
            };

            let prior_contiguous = state.contiguous_index;
            self.scan_chunked(anchor.from_block, anchor.to_block)
                .await?;

            let next_state =
                advance_contiguous_index(&self.pool, self.partitions.token_id()).await?;
            if next_state.contiguous_index <= prior_contiguous {
                return Ok(next_state);
            }

            state = next_state;
        }

        Ok(state)
    }

    async fn fetch_block_hashes(&self, events: &[IndexedEvent]) -> Result<Vec<(u64, B256)>> {
        use std::collections::HashSet;

        let mut unique = HashSet::new();
        for event in events {
            if event.eth_block_number == 0 {
                continue;
            }
            unique.insert(event.eth_block_number);
        }

        if unique.is_empty() {
            return Ok(Vec::new());
        }

        let futs: Vec<_> = unique
            .into_iter()
            .map(|number| async move {
                let hash = self
                    .contract
                    .block_hash_by_number(number)
                    .await
                    .map_err(|err| EventIndexerError::contract("block_hash_by_number", err))?;
                Ok((number, hash))
            })
            .collect();

        let results: Vec<Result<(u64, B256)>> = futures::future::join_all(futs).await;
        results.into_iter().collect()
    }

    async fn detect_reorg(&self, state: &IndexerState) -> Result<Option<u64>> {
        let window = self.config.reorg_check_window();
        if window == 0 {
            return Ok(None);
        }

        if state.last_synced_block <= self.deployed_block_number {
            return Ok(None);
        }

        let from_block = state
            .last_synced_block
            .saturating_sub(window)
            .max(self.deployed_block_number);
        let from_i64 = to_i64(from_block, "reorg from_block")?;
        let to_i64 = to_i64(state.last_synced_block, "reorg to_block")?;

        let sql = format!(
            r#"
            SELECT block_number, block_hash
            FROM {blocks_table}
            WHERE token_id = $1 AND block_number BETWEEN $2 AND $3
            ORDER BY block_number DESC
            "#,
            blocks_table = BLOCKS_TABLE,
        );

        let rows: Vec<BlockHashRow> = sqlx::query_as(&sql)
            .bind(self.partitions.token_id())
            .bind(from_i64)
            .bind(to_i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|err| EventIndexerError::database("load recent block hashes", err))?;

        // Walk from the chain tip backward. When we find a mismatch (or a
        // block that no longer exists), record it as the earliest known reorged
        // block and keep scanning to find the true fork point. Once we hit a
        // match, we know the fork is between the matching block and the
        // earliest mismatch we found.
        let mut earliest_mismatch: Option<u64> = None;
        for row in rows {
            let number = to_u64(row.block_number, "indexed block number")?;
            let stored = B256::from_slice(row.block_hash.as_slice());
            let mismatched = match self.contract.block_hash_by_number(number).await {
                Ok(current) => current != stored,
                Err(err) => {
                    if is_block_not_found(&err) {
                        // Block no longer exists on the chain — treat as reorged.
                        true
                    } else {
                        return Err(EventIndexerError::contract("block_hash_by_number", err));
                    }
                }
            };
            if mismatched {
                earliest_mismatch = Some(number);
            } else if earliest_mismatch.is_some() {
                // Found a matching block below the mismatch — fork point located.
                break;
            }
        }

        Ok(earliest_mismatch)
    }

    async fn rollback_to_block(&self, reorg_block: u64) -> Result<()> {
        let target_last_synced = reorg_block
            .saturating_sub(1)
            .max(self.deployed_block_number);

        let mut tx: Transaction<Postgres> = self
            .pool
            .begin()
            .await
            .map_err(|err| EventIndexerError::database("begin reorg rollback", err))?;

        let delete_events_sql = format!(
            "DELETE FROM {events_table} WHERE token_id = $1 AND eth_block_number >= $2",
            events_table = EVENTS_TABLE,
        );
        sqlx::query(&delete_events_sql)
            .bind(self.partitions.token_id())
            .bind(to_i64(reorg_block, "reorg block for delete")?)
            .execute(&mut *tx)
            .await
            .map_err(|err| EventIndexerError::database("delete reorged events", err))?;

        let delete_blocks_sql = format!(
            "DELETE FROM {blocks_table} WHERE token_id = $1 AND block_number >= $2",
            blocks_table = BLOCKS_TABLE,
        );
        sqlx::query(&delete_blocks_sql)
            .bind(self.partitions.token_id())
            .bind(to_i64(reorg_block, "reorg block for delete")?)
            .execute(&mut *tx)
            .await
            .map_err(|err| EventIndexerError::database("delete reorged blocks", err))?;

        // Recompute the contiguous index from remaining events, stopping at the first gap.
        let contiguous_sql = format!(
            r#"
            WITH ordered AS (
                SELECT event_index,
                       eth_block_number,
                       ROW_NUMBER() OVER (ORDER BY event_index) - 1 AS expected
                FROM {events_table}
                WHERE token_id = $1 AND eth_block_number < $2
            ),
            gap AS (
                SELECT MIN(expected) AS gap_at
                FROM ordered
                WHERE event_index != expected
            ),
            contig AS (
                SELECT
                    CASE
                        WHEN NOT EXISTS (SELECT 1 FROM ordered) THEN -1
                        WHEN (SELECT gap_at FROM gap) IS NULL THEN (SELECT MAX(event_index) FROM ordered)
                        ELSE (SELECT gap_at FROM gap) - 1
                    END AS contiguous_index
            )
            SELECT contiguous_index,
                   (SELECT eth_block_number
                    FROM ordered
                    WHERE event_index = contig.contiguous_index) AS contiguous_block
            FROM contig
            "#,
            events_table = EVENTS_TABLE,
        );
        let row: (i64, Option<i64>) = sqlx::query_as(&contiguous_sql)
            .bind(self.partitions.token_id())
            .bind(to_i64(reorg_block, "reorg block for contiguous query")?)
            .fetch_one(&mut *tx)
            .await
            .map_err(|err| {
                EventIndexerError::database("query contiguous index before reorg block", err)
            })?;
        let new_contiguous_index = row.0;
        let new_contiguous_block = row.1;

        let update_state_sql = format!(
            r#"
            UPDATE {state_table}
            SET last_synced_block = $1,
                contiguous_index = $2,
                contiguous_block = $3,
                last_seen_contract_index = NULL,
                updated_at = NOW()
            WHERE token_id = $4
            "#,
            state_table = STATE_TABLE,
        );
        sqlx::query(&update_state_sql)
            .bind(to_i64(target_last_synced, "reorg last_synced_block")?)
            .bind(new_contiguous_index)
            .bind(new_contiguous_block)
            .bind(self.partitions.token_id())
            .execute(&mut *tx)
            .await
            .map_err(|err| EventIndexerError::database("reset state after reorg", err))?;

        tx.commit()
            .await
            .map_err(|err| EventIndexerError::database("commit reorg rollback", err))?;

        Ok(())
    }
}

fn is_block_not_found(err: &ContractError) -> bool {
    matches!(err, ContractError::BlockNotFound(_))
}

struct GapAnchor {
    from_block: u64,
    to_block: u64,
}

struct IndexerState {
    contiguous_index: i64,
    contiguous_block: Option<u64>,
    last_synced_block: u64,
    _last_seen_contract_index: Option<u64>,
}

#[derive(FromRow)]
struct IndexerStateRow {
    contiguous_index: i64,
    contiguous_block: Option<i64>,
    last_synced_block: i64,
    last_seen_contract_index: Option<i64>,
}

#[derive(FromRow)]
struct EventSummaryRow {
    event_index: i64,
    eth_block_number: i64,
}

#[derive(FromRow)]
struct BlockHashRow {
    block_number: i64,
    block_hash: Vec<u8>,
}

struct PreparedEvent {
    index_i64: i64,
    block_i64: i64,
    from: Vec<u8>,
    to: Vec<u8>,
    value_bytes: [u8; VALUE_BYTES],
}

#[derive(Clone, Debug)]
struct EventIndexerPartitions {
    token_id: i64,
    events_partition: String,
    blocks_partition: String,
    state_partition: String,
}

impl EventIndexerPartitions {
    fn new(token_id: i64) -> Result<Self> {
        if token_id <= 0 {
            return Err(EventIndexerError::InvalidTokenId { token_id });
        }
        let suffix = format!("p{token_id}");
        let events_partition = format!("{EVENTS_TABLE}_{suffix}");
        let blocks_partition = format!("{BLOCKS_TABLE}_{suffix}");
        let state_partition = format!("{STATE_TABLE}_{suffix}");
        Ok(Self {
            token_id,
            events_partition,
            blocks_partition,
            state_partition,
        })
    }

    fn token_id(&self) -> i64 {
        self.token_id
    }

    async fn ensure(&self, pool: &PgPool) -> Result<()> {
        let events_sql = format!(
            "CREATE TABLE IF NOT EXISTS {partition} PARTITION OF {parent} FOR VALUES IN ({value})",
            partition = self.events_partition,
            parent = EVENTS_TABLE,
            value = self.token_id,
        );
        sqlx::query(&events_sql)
            .execute(pool)
            .await
            .map_err(|err| EventIndexerError::database("ensure events partition", err))?;

        let state_sql = format!(
            "CREATE TABLE IF NOT EXISTS {partition} PARTITION OF {parent} FOR VALUES IN ({value})",
            partition = self.state_partition,
            parent = STATE_TABLE,
            value = self.token_id,
        );
        sqlx::query(&state_sql)
            .execute(pool)
            .await
            .map_err(|err| EventIndexerError::database("ensure state partition", err))?;

        let blocks_sql = format!(
            "CREATE TABLE IF NOT EXISTS {partition} PARTITION OF {parent} FOR VALUES IN ({value})",
            partition = self.blocks_partition,
            parent = BLOCKS_TABLE,
            value = self.token_id,
        );
        sqlx::query(&blocks_sql)
            .execute(pool)
            .await
            .map_err(|err| EventIndexerError::database("ensure blocks partition", err))?;

        Ok(())
    }
}

/// Detects whether a contract error indicates that the requested block range was too large.
///
/// Different RPC providers return different error messages when a block range query exceeds
/// their limits. This function matches known patterns to enable adaptive span reduction.
///
/// Known patterns (case-insensitive):
/// - "invalid block range" - Generic provider error
/// - "block range params" - Some providers use this phrasing
/// - "block range is too large" - Explicit size rejection
/// - "exceed maximum block range" - Alchemy, Infura style
/// - "query returned more than" - Result size limits
/// - "too many results" - Generic result limit
/// - "eth_getLogs" + "limit" - Log query specific limits
/// - "range too large" - Shortened variant
///
/// If you encounter a new error pattern that should trigger span reduction,
/// add it here and document the provider that uses it.
fn is_invalid_block_range_error(err: &ContractError) -> bool {
    let message = err.to_string().to_ascii_lowercase();
    message.contains("invalid block range")
        || message.contains("block range params")
        || message.contains("block range is too large")
        || message.contains("exceed maximum block range")
        || message.contains("query returned more than")
        || message.contains("too many results")
        || message.contains("range too large")
        || (message.contains("eth_getlogs") && message.contains("limit"))
}

/// Checks if an error might be a block range error that we failed to recognize.
/// Returns true if the error looks like it could be provider-related but doesn't match known patterns.
fn is_potential_unrecognized_block_range_error(err: &ContractError) -> bool {
    let message = err.to_string().to_ascii_lowercase();
    // Common keywords that suggest provider/RPC issues related to queries
    (message.contains("block") || message.contains("range") || message.contains("limit"))
        && (message.contains("error") || message.contains("exceed") || message.contains("too"))
}

async fn ensure_token_record(pool: &PgPool, metadata: &TokenMetadata) -> Result<i64> {
    let chain_id = to_i64(metadata.chain_id, "chain_id")?;
    let token_address = metadata.token_address.as_slice();
    let verifier_address = metadata.verifier_address.as_slice();

    let sql = format!(
        r#"
        INSERT INTO {tokens_table} (token_address, verifier_address, chain_id)
        VALUES ($1, $2, $3)
        ON CONFLICT (token_address, chain_id)
        DO UPDATE
        SET verifier_address = EXCLUDED.verifier_address,
            updated_at = NOW()
        RETURNING id
        "#,
        tokens_table = TOKENS_TABLE,
    );

    let id = sqlx::query_scalar::<_, i64>(&sql)
        .bind(token_address)
        .bind(verifier_address)
        .bind(chain_id)
        .fetch_one(pool)
        .await
        .map_err(|err| EventIndexerError::database("ensure indexed token record", err))?;

    Ok(id)
}

async fn ensure_state_row(
    pool: &PgPool,
    token_id: i64,
    deployed_block_number: u64,
) -> Result<IndexerState> {
    let initial_block = to_i64(
        deployed_block_number,
        "deployed_block_number for initial state",
    )?;

    let insert_sql = format!(
        r#"
        INSERT INTO {state_table} (
            token_id,
            contiguous_index,
            contiguous_block,
            last_synced_block,
            last_seen_contract_index
        )
        VALUES ($1, -1, NULL, $2, NULL)
        ON CONFLICT (token_id) DO NOTHING
        "#,
        state_table = STATE_TABLE,
    );
    sqlx::query(&insert_sql)
        .bind(token_id)
        .bind(initial_block)
        .execute(pool)
        .await
        .map_err(|err| EventIndexerError::database("insert initial indexer state", err))?;

    let select_sql = format!(
        r#"
        SELECT contiguous_index, contiguous_block, last_synced_block, last_seen_contract_index
        FROM {state_table}
        WHERE token_id = $1
        "#,
        state_table = STATE_TABLE,
    );
    let mut row = sqlx::query_as::<_, IndexerStateRow>(&select_sql)
        .bind(token_id)
        .fetch_one(pool)
        .await
        .map_err(|err| EventIndexerError::database("load indexer state", err))?;

    if row.last_synced_block < initial_block {
        let update_sql = format!(
            r#"
            UPDATE {state_table}
            SET last_synced_block = $1,
                updated_at = NOW()
            WHERE token_id = $2
            "#,
            state_table = STATE_TABLE,
        );
        sqlx::query(&update_sql)
            .bind(initial_block)
            .bind(token_id)
            .execute(pool)
            .await
            .map_err(|err| EventIndexerError::database("update initial last_synced_block", err))?;

        row.last_synced_block = initial_block;
    }

    indexer_state_from_row(row)
}

async fn persist_sync_watermark(
    pool: &PgPool,
    token_id: i64,
    latest_block: u64,
    contract_next_index: u64,
) -> Result<()> {
    let last_block = to_i64(latest_block, "last_synced_block")?;
    let next_index = to_i64(contract_next_index, "last_seen_contract_index")?;

    let update_sql = format!(
        r#"
        UPDATE {state_table}
        SET last_synced_block = $1,
            last_seen_contract_index = $2,
            updated_at = NOW()
        WHERE token_id = $3
        "#,
        state_table = STATE_TABLE,
    );
    sqlx::query(&update_sql)
        .bind(last_block)
        .bind(next_index)
        .bind(token_id)
        .execute(pool)
        .await
        .map_err(|err| EventIndexerError::database("update sync watermark", err))?;

    Ok(())
}

async fn advance_contiguous_index(pool: &PgPool, token_id: i64) -> Result<IndexerState> {
    let mut tx: Transaction<Postgres> = pool.begin().await.map_err(|err| {
        EventIndexerError::database("begin contiguous advancement transaction", err)
    })?;

    let lock_sql = format!(
        r#"
        SELECT contiguous_index, contiguous_block, last_synced_block, last_seen_contract_index
        FROM {state_table}
        WHERE token_id = $1
        FOR UPDATE
        "#,
        state_table = STATE_TABLE,
    );
    let row = sqlx::query_as::<_, (i64, Option<i64>, i64, Option<i64>)>(&lock_sql)
        .bind(token_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| EventIndexerError::database("lock indexer state", err))?;

    let mut contiguous_index = row.0;
    let mut contiguous_block = row.1;
    let last_synced_block = row.2;
    let last_seen_contract_index = row.3;

    let mut advanced = false;
    loop {
        let next_index = contiguous_index + 1;
        let limit = ADVANCE_BATCH_SIZE.max(1);
        let events_sql = format!(
            r#"
            SELECT event_index, eth_block_number
            FROM {events_table}
            WHERE token_id = $1 AND event_index >= $2
            ORDER BY event_index ASC
            LIMIT $3
            "#,
            events_table = EVENTS_TABLE,
        );
        let rows = sqlx::query_as::<_, EventSummaryRow>(&events_sql)
            .bind(token_id)
            .bind(next_index)
            .bind(limit)
            .fetch_all(&mut *tx)
            .await
            .map_err(|err| {
                EventIndexerError::database("probe next contiguous events batch", err)
            })?;

        if rows.is_empty() {
            break;
        }

        for event_row in rows {
            let expected = contiguous_index + 1;
            if event_row.event_index != expected {
                // gap encountered; stop advancing
                break;
            }
            contiguous_index = event_row.event_index;
            contiguous_block = Some(event_row.eth_block_number);
            advanced = true;
        }

        // if we didn't fill the batch, no need to loop further
        if contiguous_index + 1 < next_index + limit {
            break;
        }
    }

    if advanced {
        let update_sql = format!(
            r#"
            UPDATE {state_table}
            SET contiguous_index = $1,
                contiguous_block = $2,
                updated_at = NOW()
            WHERE token_id = $3
            "#,
            state_table = STATE_TABLE,
        );
        sqlx::query(&update_sql)
            .bind(contiguous_index)
            .bind(contiguous_block)
            .bind(token_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| EventIndexerError::database("update contiguous index", err))?;
    }

    tx.commit()
        .await
        .map_err(|err| EventIndexerError::database("commit contiguous advancement", err))?;

    indexer_state_from_row(IndexerStateRow {
        contiguous_index,
        contiguous_block,
        last_synced_block,
        last_seen_contract_index,
    })
}

async fn find_gap_anchor(
    pool: &PgPool,
    token_id: i64,
    state: &IndexerState,
    target_last_index: u64,
    latest_block: u64,
    deployed_block_number: u64,
) -> Result<Option<GapAnchor>> {
    let current = state.contiguous_index;
    let gap_start = (current + 1).max(0) as u64;

    if gap_start > target_last_index {
        return Ok(None);
    }

    let next_sql = format!(
        r#"
        SELECT event_index, eth_block_number
        FROM {events_table}
        WHERE token_id = $1 AND event_index >= $2
        ORDER BY event_index ASC
        LIMIT 1
        "#,
        events_table = EVENTS_TABLE,
    );
    let next_known = sqlx::query_as::<_, EventSummaryRow>(&next_sql)
        .bind(token_id)
        .bind(to_i64(gap_start, "gap start index")?)
        .fetch_optional(pool)
        .await
        .map_err(|err| EventIndexerError::database("locate next known event for gap", err))?;

    let (_, to_block) = match next_known {
        Some(row) => {
            let row_index = to_u64(row.event_index, "next known event index")?;
            if row_index <= gap_start {
                return Ok(None);
            }
            let row_block = to_u64(row.eth_block_number, "next known event block")?;
            (row_index - 1, row_block)
        }
        None => (target_last_index, latest_block),
    };

    let from_block = state
        .contiguous_block
        .unwrap_or(deployed_block_number)
        .max(deployed_block_number);

    if from_block > to_block {
        return Ok(None);
    }

    Ok(Some(GapAnchor {
        from_block,
        to_block,
    }))
}

async fn insert_events(pool: &PgPool, token_id: i64, events: &[IndexedEvent]) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }

    let mut prepared = Vec::with_capacity(events.len());
    for event in events {
        let index_i64 = to_i64(event.event_index, "event index")?;
        let block_i64 = to_i64(event.eth_block_number, "event block number")?;
        prepared.push(PreparedEvent {
            index_i64,
            block_i64,
            from: event.from.as_slice().to_vec(),
            to: event.to.as_slice().to_vec(),
            value_bytes: u256_to_bytes(&event.value),
        });
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|err| EventIndexerError::database("begin events insert transaction", err))?;

    let mut builder = QueryBuilder::<Postgres>::new(format!(
        "INSERT INTO {events_table} (token_id, event_index, from_address, to_address, value, eth_block_number) ",
        events_table = EVENTS_TABLE,
    ));
    builder.push_values(&prepared, |mut b, event| {
        b.push_bind(token_id);
        b.push_bind(event.index_i64);
        b.push_bind(event.from.as_slice());
        b.push_bind(event.to.as_slice());
        b.push_bind(event.value_bytes.as_slice());
        b.push_bind(event.block_i64);
    });
    builder.push(
        " ON CONFLICT (token_id, event_index) DO UPDATE \
         SET from_address = EXCLUDED.from_address, \
             to_address = EXCLUDED.to_address, \
             value = EXCLUDED.value, \
             eth_block_number = EXCLUDED.eth_block_number",
    );

    builder
        .build()
        .execute(&mut *tx)
        .await
        .map_err(|err| EventIndexerError::database("insert events batch", err))?;

    tx.commit()
        .await
        .map_err(|err| EventIndexerError::database("commit events insert transaction", err))?;

    Ok(())
}

async fn upsert_indexed_blocks(pool: &PgPool, token_id: i64, blocks: &[(u64, B256)]) -> Result<()> {
    if blocks.is_empty() {
        return Ok(());
    }

    let mut prepared = Vec::with_capacity(blocks.len());
    for (block_number, block_hash) in blocks {
        let block_i64 = to_i64(*block_number, "event block number")?;
        prepared.push((block_i64, *block_hash));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|err| EventIndexerError::database("begin blocks insert transaction", err))?;

    let mut builder = QueryBuilder::<Postgres>::new(format!(
        "INSERT INTO {blocks_table} (token_id, block_number, block_hash) ",
        blocks_table = BLOCKS_TABLE,
    ));
    builder.push_values(&prepared, |mut b, (block_i64, block_hash)| {
        b.push_bind(token_id);
        b.push_bind(*block_i64);
        b.push_bind(block_hash.as_slice());
    });
    builder.push(
        " ON CONFLICT (token_id, block_number) DO UPDATE \
         SET block_hash = EXCLUDED.block_hash",
    );

    builder
        .build()
        .execute(&mut *tx)
        .await
        .map_err(|err| EventIndexerError::database("upsert blocks batch", err))?;

    tx.commit()
        .await
        .map_err(|err| EventIndexerError::database("commit blocks insert transaction", err))?;

    Ok(())
}

async fn prune_old_blocks(
    pool: &PgPool,
    token_id: i64,
    latest_block: u64,
    reorg_check_window: u64,
) -> Result<()> {
    if reorg_check_window == 0 {
        return Ok(());
    }

    let cutoff = latest_block.saturating_sub(reorg_check_window);
    if cutoff == 0 {
        return Ok(());
    }

    let sql = format!(
        "DELETE FROM {blocks_table} WHERE token_id = $1 AND block_number < $2",
        blocks_table = BLOCKS_TABLE,
    );
    sqlx::query(&sql)
        .bind(token_id)
        .bind(to_i64(cutoff, "prune cutoff block")?)
        .execute(pool)
        .await
        .map_err(|err| EventIndexerError::database("prune old indexed blocks", err))?;

    Ok(())
}

fn indexer_state_from_row(row: IndexerStateRow) -> Result<IndexerState> {
    Ok(IndexerState {
        contiguous_index: row.contiguous_index,
        contiguous_block: opt_i64_to_u64(row.contiguous_block, "contiguous_block")?,
        last_synced_block: to_u64(row.last_synced_block, "last_synced_block")?,
        _last_seen_contract_index: opt_i64_to_u64(
            row.last_seen_contract_index,
            "last_seen_contract_index",
        )?,
    })
}

fn u256_to_bytes(value: &U256) -> [u8; VALUE_BYTES] {
    value.to_be_bytes::<VALUE_BYTES>()
}

fn to_u64(value: i64, label: &'static str) -> Result<u64> {
    u64::try_from(value).map_err(|_| EventIndexerError::I64ToU64 { label, value })
}

fn to_i64(value: u64, label: &'static str) -> Result<i64> {
    i64::try_from(value).map_err(|_| EventIndexerError::U64ToI64 { label, value })
}

fn opt_i64_to_u64(value: Option<i64>, label: &'static str) -> Result<Option<u64>> {
    match value {
        Some(v) => Ok(Some(to_u64(v, label)?)),
        None => Ok(None),
    }
}
