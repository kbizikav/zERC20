use thiserror::Error;

use zkp::nova::params::NovaError;

#[derive(Debug, Error)]
pub enum ProverError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("queue error: {0}")]
    Queue(#[from] pgmq::PgmqError),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("nova error: {0}")]
    Nova(#[from] NovaError),
    #[error("hex decode error: {0}")]
    HexDecode(#[from] hex::FromHexError),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}
