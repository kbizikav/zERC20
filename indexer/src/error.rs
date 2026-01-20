pub type Result<T> = anyhow::Result<T>;

// Re-export domain-specific errors for callers that need structured handling.
pub use crate::{events::EventIndexerError, trees::DbMerkleTreeError};
