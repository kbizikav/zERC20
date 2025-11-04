mod db;

pub use db::{
    AppendResult, DbIncrementalMerkleTree, DbMerkleTreeConfig, DbMerkleTreeError,
    HISTORY_WINDOW_RECOMMENDED, HistoricalProof,
};
