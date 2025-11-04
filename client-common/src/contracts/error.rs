use alloy::contract;
use std::error::Error as StdError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ContractError {
    #[error(transparent)]
    Contract(#[from] contract::Error),
    #[error("transport error while {action}: {source}")]
    Transport {
        action: &'static str,
        #[source]
        source: Box<dyn StdError + Send + Sync + 'static>,
    },
    #[error("event `{0}` not found in transaction logs")]
    MissingEvent(&'static str),
}

pub type ContractResult<T> = Result<T, ContractError>;

impl ContractError {
    pub fn transport<E>(action: &'static str, source: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        Self::Transport {
            action,
            source: Box::new(source),
        }
    }
}
