use std::convert::TryFrom;
use std::num::NonZeroU64;

use alloy::primitives::U256;
use api_types::indexer::IndexedEvent;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use thiserror::Error;

use client_common::contracts::{ContractError, z_erc20::ZErc20Contract};
use client_common::tokens::TokenMetadata;

pub const BLOCK_SPAN_RECOMMENDED: u64 = 5_000;
pub const FORWARD_SCAN_OVERLAP_RECOMMENDED: u64 = 10;
const VALUE_BYTES: usize = 32;
const EVENTS_TABLE: &str = "indexed_transfer_events";
const STATE_TABLE: &str = "event_indexer_state";
const TOKENS_TABLE: &str = "tokens";

pub type Result<T> = std::result::Result<T, EventIndexerError>;

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
}

impl EventIndexerConfig {
    pub fn new(block_span: u64, forward_scan_overlap: u64) -> Result<Self> {
        let Some(block_span) = NonZeroU64::new(block_span) else {
            return Err(EventIndexerError::NonPositiveConfig {
                label: "block_span",
            });
        };
        Ok(Self {
            block_span,
            forward_scan_overlap,
        })
    }

    pub fn block_span(&self) -> NonZeroU64 {
        self.block_span
    }

    pub fn forward_scan_overlap(&self) -> u64 {
        self.forward_scan_overlap
    }
}

pub struct EventIndexer {
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
    ) -> Result<Self> {
        let token_id = ensure_token_record(&pool, &metadata).await?;
        let partitions = EventIndexerPartitions::new(token_id)?;
        Ok(Self {
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

        let latest_block = self
            .contract
            .latest_block()
            .await
            .map_err(|err| EventIndexerError::contract("latest_block", err))?;
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

        Ok(())
    }

    async fn scan_chunked(&self, from_block: u64, to_block: u64) -> Result<()> {
        if from_block > to_block {
            return Ok(());
        }

        let block_span = self.config.block_span().get();
        let forward_overlap = self.config.forward_scan_overlap();
        let mut from = from_block;
        while from <= to_block {
            let to = to_block.min(from.saturating_add(block_span - 1));
            let fetched = self
                .contract
                .get_indexed_transfer_events(from, to)
                .await
                .map_err(|err| EventIndexerError::contract("get_indexed_transfer_events", err))?;

            if !fetched.is_empty() {
                insert_events(&self.pool, self.partitions.token_id(), &fetched).await?;
            }

            if to == to_block {
                break;
            }

            let next_from = to.saturating_add(1);
            from = next_from.saturating_sub(forward_overlap.min(next_from));
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

#[derive(Clone, Debug)]
struct EventIndexerPartitions {
    token_id: i64,
    events_partition: String,
    state_partition: String,
}

impl EventIndexerPartitions {
    fn new(token_id: i64) -> Result<Self> {
        if token_id <= 0 {
            return Err(EventIndexerError::InvalidTokenId { token_id });
        }
        let suffix = format!("p{token_id}");
        let events_partition = format!("{EVENTS_TABLE}_{suffix}");
        let state_partition = format!("{STATE_TABLE}_{suffix}");
        Ok(Self {
            token_id,
            events_partition,
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

        Ok(())
    }
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
        let events_sql = format!(
            r#"
            SELECT event_index, eth_block_number
            FROM {events_table}
            WHERE token_id = $1 AND event_index = $2
            "#,
            events_table = EVENTS_TABLE,
        );
        let next_row = sqlx::query_as::<_, EventSummaryRow>(&events_sql)
            .bind(token_id)
            .bind(next_index)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|err| EventIndexerError::database("probe next contiguous event", err))?;

        match next_row {
            Some(event_row) => {
                contiguous_index = event_row.event_index;
                contiguous_block = Some(event_row.eth_block_number);
                advanced = true;
            }
            None => break,
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

    let mut tx = pool
        .begin()
        .await
        .map_err(|err| EventIndexerError::database("begin events insert transaction", err))?;

    let insert_sql = format!(
        r#"
        INSERT INTO {events_table} (
            token_id,
            event_index,
            from_address,
            to_address,
            value,
            eth_block_number
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (token_id, event_index) DO UPDATE
        SET from_address = EXCLUDED.from_address,
            to_address = EXCLUDED.to_address,
            value = EXCLUDED.value,
            eth_block_number = EXCLUDED.eth_block_number
        "#,
        events_table = EVENTS_TABLE,
    );

    for event in events {
        let index = to_i64(event.event_index, "event index")?;
        let block = to_i64(event.eth_block_number, "event block number")?;
        let from = event.from.as_slice();
        let to = event.to.as_slice();
        let value_bytes = u256_to_bytes(&event.value);

        sqlx::query(&insert_sql)
            .bind(token_id)
            .bind(index)
            .bind(from)
            .bind(to)
            .bind(value_bytes.as_slice())
            .bind(block)
            .execute(&mut *tx)
            .await
            .map_err(|err| EventIndexerError::InsertEvent {
                index: event.event_index,
                source: err,
            })?;
    }

    tx.commit()
        .await
        .map_err(|err| EventIndexerError::database("commit events insert transaction", err))?;

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
