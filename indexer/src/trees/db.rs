use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;

use alloy::primitives::{Address, U256};
use ark_bn254::Fr;
use ark_ff::{BigInteger, PrimeField};
use sqlx::{PgPool, Postgres, QueryBuilder, Row, Transaction};
use thiserror::Error;

use zkp::utils::{
    convertion::{address_to_fr, u256_to_fr},
    poseidon::utils::poseidon2,
    tree::{
        bit_path::BitPath,
        gadgets::{hash_chain::hash_chain, leaf_hash::compute_leaf_hash},
        merkle_tree::MerkleProof,
    },
};

pub const HISTORY_WINDOW_RECOMMENDED: u64 = 100;
const MERKLE_NODES_TABLE: &str = "merkle_nodes_current";
const MERKLE_UPDATES_TABLE: &str = "merkle_node_updates";
const MERKLE_SNAPSHOTS_TABLE: &str = "merkle_snapshots";
const PG_DUPLICATE_TABLE: &str = "42P07";

pub type Result<T> = std::result::Result<T, DbMerkleTreeError>;

#[derive(Debug, Error)]
pub enum DbMerkleTreeError {
    #[error("invalid token id {token_id} for partitioned tables")]
    InvalidTokenId { token_id: i64 },
    #[error("merkle tree height must be positive")]
    InvalidHeight { height: u32 },
    #[error("overflow computing next leaf index")]
    LeafIndexOverflow,
    #[error("cannot prove merkle state for index 0")]
    InvalidProofTargetZero,
    #[error("merkle tree empty: proof unavailable")]
    TreeEmpty,
    #[error("target index {target} exceeds latest index {latest}")]
    TargetIndexTooHigh { target: u64, latest: u64 },
    #[error("requested index {target} exceeds retention window of {window}")]
    RetentionWindowExceeded { target: u64, window: u64 },
    #[error("leaf index {leaf_index} not present at tree index {target_index}")]
    LeafIndexOutOfBounds { leaf_index: u64, target_index: u64 },
    #[error("token id {token_id} not present in tokens table")]
    TokenNotFound { token_id: i64 },
    #[error("missing root hash for target index {index}")]
    MissingRoot { index: u64 },
    #[error("missing hash chain for target index {index}")]
    MissingHashChain { index: u64 },
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
    #[error("invalid Fr byte length: expected 32, got {len}")]
    InvalidFrBytes { len: usize },
    #[error("invalid U256 byte length: expected 32, got {len}")]
    InvalidU256Bytes { len: usize },
    #[error("invalid bit path byte length: expected 12, got {len}")]
    InvalidBitPathBytes { len: usize },
    #[error("{message}")]
    InvalidConfig { message: &'static str },
}

impl DbMerkleTreeError {
    fn database(action: &'static str, source: sqlx::Error) -> Self {
        Self::Database { action, source }
    }
}

#[derive(Debug, Clone)]
pub struct DbMerkleTreeConfig {
    history_window: NonZeroU64,
}

impl DbMerkleTreeConfig {
    pub fn new(history_window: u64) -> Result<Self> {
        let Some(history_window) = NonZeroU64::new(history_window) else {
            return Err(DbMerkleTreeError::InvalidConfig {
                message: "history_window must be greater than zero",
            });
        };
        Ok(Self { history_window })
    }

    pub fn history_window(&self) -> NonZeroU64 {
        self.history_window
    }
}

#[derive(Debug, Clone)]
pub struct AppendResult {
    pub index: u64,
    pub leaf_index: u64,
    pub root: Fr,
    pub hash_chain: U256,
}

#[derive(Debug, Clone)]
pub struct HistoricalProof {
    pub target_index: u64,
    pub leaf_index: u64,
    pub root: Fr,
    pub hash_chain: U256,
    pub proof: MerkleProof,
}

#[derive(Debug)]
struct NodeUpdateRow {
    path_bytes: [u8; 12],
    old_bytes: Vec<u8>,
    new_bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
struct TreePartitions {
    token_id: i64,
    nodes_partition: String,
    updates_partition: String,
    snapshots_partition: String,
}

fn is_duplicate_table_error(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db_err)
        if db_err
            .code()
            .map(|code| code == PG_DUPLICATE_TABLE)
            .unwrap_or(false))
}

async fn execute_partition_statement(pool: &PgPool, sql: &str, action: &'static str) -> Result<()> {
    match sqlx::query(sql).execute(pool).await {
        Ok(_) => Ok(()),
        Err(err) if is_duplicate_table_error(&err) => Ok(()),
        Err(err) => Err(DbMerkleTreeError::database(action, err)),
    }
}

impl TreePartitions {
    fn new(token_id: i64) -> Result<Self> {
        if token_id <= 0 {
            return Err(DbMerkleTreeError::InvalidTokenId { token_id });
        }
        let suffix = format!("p{token_id}");
        Ok(Self {
            token_id,
            nodes_partition: format!("{MERKLE_NODES_TABLE}_{suffix}"),
            updates_partition: format!("{MERKLE_UPDATES_TABLE}_{suffix}"),
            snapshots_partition: format!("{MERKLE_SNAPSHOTS_TABLE}_{suffix}"),
        })
    }

    fn token_id(&self) -> i64 {
        self.token_id
    }

    async fn ensure(&self, pool: &PgPool) -> Result<()> {
        let nodes_sql = format!(
            "CREATE TABLE IF NOT EXISTS {partition} PARTITION OF {parent} FOR VALUES IN ({value})",
            partition = self.nodes_partition,
            parent = MERKLE_NODES_TABLE,
            value = self.token_id,
        );
        execute_partition_statement(pool, &nodes_sql, "ensure merkle nodes partition").await?;

        let updates_sql = format!(
            "CREATE TABLE IF NOT EXISTS {partition} PARTITION OF {parent} FOR VALUES IN ({value})",
            partition = self.updates_partition,
            parent = MERKLE_UPDATES_TABLE,
            value = self.token_id,
        );
        execute_partition_statement(pool, &updates_sql, "ensure merkle updates partition").await?;

        let snapshots_sql = format!(
            "CREATE TABLE IF NOT EXISTS {partition} PARTITION OF {parent} FOR VALUES IN ({value})",
            partition = self.snapshots_partition,
            parent = MERKLE_SNAPSHOTS_TABLE,
            value = self.token_id,
        );
        execute_partition_statement(pool, &snapshots_sql, "ensure merkle snapshots partition")
            .await?;

        Ok(())
    }
}

pub struct DbIncrementalMerkleTree {
    pool: PgPool,
    partitions: TreePartitions,
    height: u32,
    zero_hashes: Vec<Fr>,
    history_window: NonZeroU64,
}

impl DbIncrementalMerkleTree {
    pub async fn new(
        pool: PgPool,
        token_id: i64,
        height: u32,
        config: DbMerkleTreeConfig,
    ) -> Result<Self> {
        if height == 0 {
            return Err(DbMerkleTreeError::InvalidHeight { height });
        }

        let partitions = TreePartitions::new(token_id)?;
        partitions.ensure(&pool).await?;

        let zero_hashes = compute_zero_hashes(height);
        Ok(Self {
            pool,
            partitions,
            height,
            zero_hashes,
            history_window: config.history_window(),
        })
    }

    pub fn zero_root(&self) -> Fr {
        *self
            .zero_hashes
            .last()
            .expect("zero hashes populated during construction")
    }

    pub async fn append_leaf(&self, address: Address, value: U256) -> Result<AppendResult> {
        let mut tx = self.pool.begin().await.map_err(|err| {
            DbMerkleTreeError::database("begin transaction for merkle append", err)
        })?;

        self.lock_token_row(&mut tx).await?;

        let latest_index = self.latest_index_internal(&mut tx).await?;
        let next_index = latest_index + 1;
        let leaf_index = next_index
            .checked_sub(1)
            .ok_or(DbMerkleTreeError::LeafIndexOverflow)?;

        let prev_hash_chain = self
            .latest_hash_chain_internal(&mut tx)
            .await?
            .unwrap_or(U256::ZERO);

        let leaf_hash = compute_leaf_hash(address_to_fr(address), u256_to_fr(value));
        let mut node_hash = leaf_hash;
        let next_index_i64 =
            i64::try_from(next_index).map_err(|_| DbMerkleTreeError::U64ToI64 {
                label: "merkle index during append",
                value: next_index,
            })?;

        let mut fetch_positions = HashSet::new();
        let mut cursor_path = BitPath::new(self.height, leaf_index);
        fetch_positions.insert(cursor_path);
        for _ in 0..self.height {
            fetch_positions.insert(cursor_path.sibling());
            let mut parent = cursor_path;
            parent.pop();
            fetch_positions.insert(parent);
            cursor_path = parent;
            if cursor_path.is_empty() {
                break;
            }
        }

        let position_list: Vec<BitPath> = fetch_positions.into_iter().collect();
        let mut existing_nodes = self.load_node_hashes(&mut tx, &position_list).await?;

        let mut planned_updates: Vec<NodeUpdateRow> = Vec::with_capacity(self.height as usize + 1);

        let mut current_path = BitPath::new(self.height, leaf_index);
        for _ in 0..self.height {
            let zero = self.zero_hash_for_path(current_path);
            let old_hash = existing_nodes.get(&current_path).copied().unwrap_or(zero);

            if old_hash != node_hash {
                planned_updates.push(NodeUpdateRow {
                    path_bytes: current_path.to_bytes(),
                    old_bytes: Vec::from(fr_to_bytes(old_hash)),
                    new_bytes: Vec::from(fr_to_bytes(node_hash)),
                });
                existing_nodes.insert(current_path, node_hash);
            }

            let sibling_path = current_path.sibling();
            let sibling_hash = existing_nodes
                .get(&sibling_path)
                .copied()
                .unwrap_or(self.zero_hash_for_path(sibling_path));

            let is_left = (current_path.value() & 1) == 0;
            let (left, right) = if is_left {
                (node_hash, sibling_hash)
            } else {
                (sibling_hash, node_hash)
            };

            node_hash = poseidon2(left, right);
            let mut parent_path = current_path;
            parent_path.pop();
            current_path = parent_path;
        }

        let root_path = BitPath::default();
        let root_old = existing_nodes
            .get(&root_path)
            .copied()
            .unwrap_or(self.zero_hash_for_path(root_path));
        if root_old != node_hash {
            planned_updates.push(NodeUpdateRow {
                path_bytes: root_path.to_bytes(),
                old_bytes: Vec::from(fr_to_bytes(root_old)),
                new_bytes: Vec::from(fr_to_bytes(node_hash)),
            });
            existing_nodes.insert(root_path, node_hash);
        }

        if !planned_updates.is_empty() {
            let mut updates_builder = QueryBuilder::<Postgres>::new(format!(
                "INSERT INTO {table} (token_id, tree_index, node_path, old_hash, new_hash)",
                table = MERKLE_UPDATES_TABLE,
            ));
            updates_builder.push_values(&planned_updates, |mut b, update| {
                b.push_bind(self.partitions.token_id());
                b.push_bind(next_index_i64);
                b.push_bind(update.path_bytes.as_slice());
                b.push_bind(update.old_bytes.as_slice());
                b.push_bind(update.new_bytes.as_slice());
            });
            updates_builder
                .build()
                .execute(tx.as_mut())
                .await
                .map_err(|err| {
                    DbMerkleTreeError::database("write merkle update rows batch", err)
                })?;

            let mut nodes_builder = QueryBuilder::<Postgres>::new(format!(
                "INSERT INTO {table} (token_id, node_path, hash, updated_at_index)",
                table = MERKLE_NODES_TABLE,
            ));
            nodes_builder.push_values(&planned_updates, |mut b, update| {
                b.push_bind(self.partitions.token_id());
                b.push_bind(update.path_bytes.as_slice());
                b.push_bind(update.new_bytes.as_slice());
                b.push_bind(next_index_i64);
            });
            nodes_builder.push(
                " ON CONFLICT (token_id, node_path)
                  DO UPDATE SET hash = EXCLUDED.hash,
                                updated_at_index = EXCLUDED.updated_at_index,
                                updated_at = NOW()",
            );
            nodes_builder
                .build()
                .execute(tx.as_mut())
                .await
                .map_err(|err| DbMerkleTreeError::database("upsert merkle node hash batch", err))?;
        }

        let new_hash_chain = hash_chain(prev_hash_chain, address, value);
        let root_bytes = fr_to_bytes(node_hash);
        let hash_chain_bytes = new_hash_chain.to_be_bytes::<32>();
        sqlx::query(&format!(
            "INSERT INTO {table} (token_id, tree_index, root_hash, hash_chain) VALUES ($1, $2, $3, $4)",
            table = MERKLE_SNAPSHOTS_TABLE,
        ))
        .bind(self.partitions.token_id())
        .bind(next_index_i64)
        .bind(root_bytes.as_slice())
        .bind(hash_chain_bytes.as_slice())
        .execute(tx.as_mut())
        .await
        .map_err(|err| DbMerkleTreeError::database("insert merkle snapshot", err))?;

        let history_window = self.history_window.get();
        let gc_threshold = next_index.saturating_sub(history_window);
        let gc_threshold_i64 =
            i64::try_from(gc_threshold).map_err(|_| DbMerkleTreeError::U64ToI64 {
                label: "gc threshold",
                value: gc_threshold,
            })?;
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE token_id = $1 AND tree_index <= $2",
            table = MERKLE_UPDATES_TABLE,
        ))
        .bind(self.partitions.token_id())
        .bind(gc_threshold_i64)
        .execute(tx.as_mut())
        .await
        .map_err(|err| DbMerkleTreeError::database("prune stale merkle updates", err))?;

        tx.commit()
            .await
            .map_err(|err| DbMerkleTreeError::database("commit merkle append transaction", err))?;

        Ok(AppendResult {
            index: next_index,
            leaf_index,
            root: node_hash,
            hash_chain: new_hash_chain,
        })
    }

    pub async fn prove(&self, target_index: u64, leaf_index: u64) -> Result<HistoricalProof> {
        let mut proofs = self
            .prove_many(target_index, std::slice::from_ref(&leaf_index))
            .await?;
        Ok(proofs
            .pop()
            .expect("prove_many must return one proof for a single leaf"))
    }

    pub async fn prove_many(
        &self,
        target_index: u64,
        leaf_indices: &[u64],
    ) -> Result<Vec<HistoricalProof>> {
        if leaf_indices.is_empty() {
            return Ok(Vec::new());
        }
        if target_index == 0 {
            return Err(DbMerkleTreeError::InvalidProofTargetZero);
        }

        let mut tx = self.pool.begin().await.map_err(|err| {
            DbMerkleTreeError::database("begin transaction for merkle proof batch", err)
        })?;

        let latest_index = self.latest_index_internal(&mut tx).await?;
        if latest_index == 0 {
            return Err(DbMerkleTreeError::TreeEmpty);
        }
        if target_index > latest_index {
            return Err(DbMerkleTreeError::TargetIndexTooHigh {
                target: target_index,
                latest: latest_index,
            });
        }

        let history_window = self.history_window.get();
        let delta = latest_index - target_index;
        if delta > history_window {
            return Err(DbMerkleTreeError::RetentionWindowExceeded {
                target: target_index,
                window: history_window,
            });
        }

        for &leaf_index in leaf_indices {
            if leaf_index >= target_index {
                return Err(DbMerkleTreeError::LeafIndexOutOfBounds {
                    leaf_index,
                    target_index,
                });
            }
        }

        let overlay = self
            .load_overlay(&mut tx, target_index + 1, latest_index)
            .await?;
        let root = self.root_at_internal(&mut tx, target_index).await?.ok_or(
            DbMerkleTreeError::MissingRoot {
                index: target_index,
            },
        )?;
        let hash_chain = self
            .hash_chain_at_internal(&mut tx, target_index)
            .await?
            .ok_or(DbMerkleTreeError::MissingHashChain {
                index: target_index,
            })?;

        let mut prefetch_paths = HashSet::new();
        for &leaf_index in leaf_indices {
            let mut path = BitPath::new(self.height, leaf_index);
            for _ in 0..self.height {
                let sibling = path.sibling();
                if !overlay.contains_key(&sibling) {
                    prefetch_paths.insert(sibling);
                }
                path.pop();
            }
        }

        let mut cache: HashMap<BitPath, Fr> = HashMap::new();
        if !prefetch_paths.is_empty() {
            let to_fetch: Vec<BitPath> = prefetch_paths.iter().copied().collect();
            let fetched = self.load_node_hashes(&mut tx, &to_fetch).await?;
            cache.extend(fetched);
        }

        tx.commit()
            .await
            .map_err(|err| DbMerkleTreeError::database("commit merkle proof batch", err))?;

        let mut proofs = Vec::with_capacity(leaf_indices.len());
        for &leaf_index in leaf_indices {
            let mut siblings = Vec::with_capacity(self.height as usize);
            let mut path = BitPath::new(self.height, leaf_index);

            for _ in 0..self.height {
                let sibling_path = path.sibling();
                let sibling_hash = if let Some(hash) = overlay.get(&sibling_path) {
                    *hash
                } else {
                    *cache
                        .entry(sibling_path)
                        .or_insert_with(|| self.zero_hash_for_path(sibling_path))
                };
                siblings.push(sibling_hash);
                path.pop();
            }

            proofs.push(HistoricalProof {
                target_index,
                leaf_index,
                root,
                hash_chain,
                proof: MerkleProof { siblings },
            });
        }

        Ok(proofs)
    }

    pub async fn latest_index(&self) -> Result<u64> {
        let mut tx = self.pool.begin().await.map_err(|err| {
            DbMerkleTreeError::database("begin transaction for latest index lookup", err)
        })?;
        let index = self.latest_index_internal(&mut tx).await?;
        tx.commit()
            .await
            .map_err(|err| DbMerkleTreeError::database("commit latest index lookup", err))?;
        Ok(index)
    }

    pub async fn root_at(&self, index: u64) -> Result<Option<Fr>> {
        let mut tx =
            self.pool.begin().await.map_err(|err| {
                DbMerkleTreeError::database("begin transaction for root lookup", err)
            })?;
        let root = self.root_at_internal(&mut tx, index).await?;
        tx.commit()
            .await
            .map_err(|err| DbMerkleTreeError::database("commit root lookup transaction", err))?;
        Ok(root)
    }

    pub async fn hash_chain_at(&self, index: u64) -> Result<Option<U256>> {
        let mut tx = self.pool.begin().await.map_err(|err| {
            DbMerkleTreeError::database("begin transaction for hash chain lookup", err)
        })?;
        let hash_chain = self.hash_chain_at_internal(&mut tx, index).await?;
        tx.commit().await.map_err(|err| {
            DbMerkleTreeError::database("commit hash chain lookup transaction", err)
        })?;
        Ok(hash_chain)
    }

    async fn latest_index_internal(&self, tx: &mut Transaction<'_, Postgres>) -> Result<u64> {
        let sql = format!(
            "SELECT tree_index FROM {table} WHERE token_id = $1 ORDER BY tree_index DESC LIMIT 1 FOR UPDATE",
            table = MERKLE_SNAPSHOTS_TABLE,
        );
        let row: Option<i64> = sqlx::query_scalar(&sql)
            .bind(self.partitions.token_id())
            .fetch_optional(tx.as_mut())
            .await
            .map_err(|err| DbMerkleTreeError::database("query latest merkle index", err))?;
        Ok(row.map(|v| v as u64).unwrap_or(0))
    }

    async fn latest_hash_chain_internal(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Option<U256>> {
        let sql = format!(
            "SELECT hash_chain FROM {table} WHERE token_id = $1 ORDER BY tree_index DESC LIMIT 1 FOR UPDATE",
            table = MERKLE_SNAPSHOTS_TABLE,
        );
        let row: Option<Vec<u8>> = sqlx::query_scalar(&sql)
            .bind(self.partitions.token_id())
            .fetch_optional(tx.as_mut())
            .await
            .map_err(|err| DbMerkleTreeError::database("load latest hash chain", err))?;
        row.map(|bytes| bytes_to_u256(bytes.as_slice())).transpose()
    }

    async fn root_at_internal(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        index: u64,
    ) -> Result<Option<Fr>> {
        let index_i64 = i64::try_from(index).map_err(|_| DbMerkleTreeError::U64ToI64 {
            label: "historical root index",
            value: index,
        })?;
        let sql = format!(
            "SELECT root_hash FROM {table} WHERE token_id = $1 AND tree_index = $2",
            table = MERKLE_SNAPSHOTS_TABLE,
        );
        let row: Option<Vec<u8>> = sqlx::query_scalar(&sql)
            .bind(self.partitions.token_id())
            .bind(index_i64)
            .fetch_optional(tx.as_mut())
            .await
            .map_err(|err| DbMerkleTreeError::database("lookup historical root", err))?;
        row.map(|bytes| bytes_to_fr(bytes.as_slice())).transpose()
    }

    async fn hash_chain_at_internal(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        index: u64,
    ) -> Result<Option<U256>> {
        let index_i64 = i64::try_from(index).map_err(|_| DbMerkleTreeError::U64ToI64 {
            label: "historical hash chain index",
            value: index,
        })?;
        let sql = format!(
            "SELECT hash_chain FROM {table} WHERE token_id = $1 AND tree_index = $2",
            table = MERKLE_SNAPSHOTS_TABLE,
        );
        let row: Option<Vec<u8>> = sqlx::query_scalar(&sql)
            .bind(self.partitions.token_id())
            .bind(index_i64)
            .fetch_optional(tx.as_mut())
            .await
            .map_err(|err| DbMerkleTreeError::database("lookup historical hash chain", err))?;
        row.map(|bytes| bytes_to_u256(bytes.as_slice())).transpose()
    }

    async fn lock_token_row(&self, tx: &mut Transaction<'_, Postgres>) -> Result<()> {
        let locked: Option<i64> =
            sqlx::query_scalar("SELECT id FROM tokens WHERE id = $1 FOR UPDATE")
                .bind(self.partitions.token_id())
                .fetch_optional(tx.as_mut())
                .await
                .map_err(|err| {
                    DbMerkleTreeError::database("lock token row for merkle append", err)
                })?;

        match locked {
            Some(_) => Ok(()),
            None => Err(DbMerkleTreeError::TokenNotFound {
                token_id: self.partitions.token_id(),
            }),
        }
    }

    async fn load_node_hashes(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        paths: &[BitPath],
    ) -> Result<HashMap<BitPath, Fr>> {
        if paths.is_empty() {
            return Ok(HashMap::new());
        }

        let mut builder = QueryBuilder::<Postgres>::new(format!(
            "SELECT node_path, hash FROM {table} WHERE token_id = ",
            table = MERKLE_NODES_TABLE,
        ));
        builder.push_bind(self.partitions.token_id());
        builder.push(" AND node_path IN (");
        for (idx, path) in paths.iter().enumerate() {
            if idx > 0 {
                builder.push(", ");
            }
            builder.push_bind(path.to_bytes().to_vec());
        }
        builder.push(")");

        let rows = builder
            .build_query_as::<(Vec<u8>, Vec<u8>)>()
            .fetch_all(tx.as_mut())
            .await
            .map_err(|err| DbMerkleTreeError::database("load merkle node hashes batch", err))?;

        let mut hashes = HashMap::with_capacity(rows.len());
        for (path_bytes, hash_bytes) in rows {
            let path = bytes_to_bit_path(path_bytes.as_slice())?;
            let hash = bytes_to_fr(hash_bytes.as_slice())?;
            hashes.insert(path, hash);
        }

        Ok(hashes)
    }

    async fn load_overlay(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        from_index: u64,
        to_index: u64,
    ) -> Result<HashMap<BitPath, Fr>> {
        if from_index > to_index {
            return Ok(HashMap::new());
        }
        let from_i64 = i64::try_from(from_index).map_err(|_| DbMerkleTreeError::U64ToI64 {
            label: "overlay lower bound",
            value: from_index,
        })?;
        let to_i64 = i64::try_from(to_index).map_err(|_| DbMerkleTreeError::U64ToI64 {
            label: "overlay upper bound",
            value: to_index,
        })?;

        let sql = format!(
            "SELECT tree_index, node_path, old_hash
             FROM {table}
             WHERE token_id = $1 AND tree_index BETWEEN $2 AND $3
             ORDER BY tree_index ASC",
            table = MERKLE_UPDATES_TABLE,
        );
        let rows = sqlx::query(&sql)
            .bind(self.partitions.token_id())
            .bind(from_i64)
            .bind(to_i64)
            .fetch_all(tx.as_mut())
            .await
            .map_err(|err| DbMerkleTreeError::database("load merkle update overlay", err))?;

        let mut overlay = HashMap::new();
        for row in rows {
            let path_bytes: Vec<u8> = row.try_get("node_path").map_err(|err| {
                DbMerkleTreeError::database("read node_path in merkle update row", err)
            })?;
            let old_hash: Vec<u8> = row.try_get("old_hash").map_err(|err| {
                DbMerkleTreeError::database("read old_hash in merkle update row", err)
            })?;

            let path = bytes_to_bit_path(path_bytes.as_slice())?;

            if overlay.contains_key(&path) {
                continue;
            }
            let hash = bytes_to_fr(old_hash.as_slice())?;
            overlay.insert(path, hash);
        }

        Ok(overlay)
    }

    fn zero_hash_for_path(&self, path: BitPath) -> Fr {
        let remaining = path.len() as usize;
        let total = self.height as usize;
        let idx = total
            .checked_sub(remaining)
            .expect("path length does not exceed height");
        *self
            .zero_hashes
            .get(idx)
            .or_else(|| self.zero_hashes.last())
            .expect("zero hashes populated")
    }
}

fn compute_zero_hashes(height: u32) -> Vec<Fr> {
    let mut hashes = Vec::with_capacity(height as usize + 1);
    let mut current = Fr::from(0u64);
    hashes.push(current);
    for _ in 0..height {
        current = poseidon2(current, current);
        hashes.push(current);
    }
    hashes
}

fn fr_to_bytes(value: Fr) -> [u8; 32] {
    let bigint = value.into_bigint();
    let mut bytes = bigint.to_bytes_be();
    if bytes.len() < 32 {
        let mut padded = vec![0u8; 32 - bytes.len()];
        padded.extend_from_slice(&bytes);
        bytes = padded;
    }
    let array: [u8; 32] = bytes.try_into().expect("fr serialization to 32 bytes");
    array
}

fn bytes_to_bit_path(bytes: &[u8]) -> Result<BitPath> {
    if bytes.len() != 12 {
        return Err(DbMerkleTreeError::InvalidBitPathBytes { len: bytes.len() });
    }
    let mut array = [0u8; 12];
    array.copy_from_slice(bytes);
    Ok(BitPath::from_bytes(array))
}

fn bytes_to_fr(bytes: &[u8]) -> Result<Fr> {
    if bytes.len() != 32 {
        return Err(DbMerkleTreeError::InvalidFrBytes { len: bytes.len() });
    }
    Ok(Fr::from_be_bytes_mod_order(bytes))
}

fn bytes_to_u256(bytes: &[u8]) -> Result<U256> {
    if bytes.len() != 32 {
        return Err(DbMerkleTreeError::InvalidU256Bytes { len: bytes.len() });
    }
    Ok(U256::from_be_slice(bytes))
}
