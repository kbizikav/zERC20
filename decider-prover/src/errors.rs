use thiserror::Error;

use zkp::nova::params::NovaError;

#[derive(Debug, Error)]
pub enum ProverError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("redis error: {0}")]
    Redis(#[from] deadpool_redis::redis::RedisError),
    #[error("redis pool error: {0}")]
    RedisPool(#[from] deadpool_redis::PoolError),
    #[error("redis pool creation error: {0}")]
    RedisPoolCreation(#[from] deadpool_redis::CreatePoolError),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("nova error: {0}")]
    Nova(#[from] NovaError),
    #[error("hex decode error: {0}")]
    HexDecode(#[from] hex::FromHexError),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}
