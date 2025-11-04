use candid::Error as CandidError;
use ic_agent::AgentError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StealthError {
    #[error("invalid address length: expected 20 bytes")]
    InvalidAddress,
    #[error("invalid derived public key")]
    InvalidDerivedPublicKey,
    #[error("encryption failure")]
    EncryptionFailed,
    #[error("decryption failure")]
    DecryptionFailed,
    #[error("IBE encryption failed: {0}")]
    IbeEncryption(String),
    #[error("IBE decryption failed: {0}")]
    IbeDecryption(String),
    #[error("invalid nonce length")]
    InvalidNonce,
    #[error("announcement ignored: {0}")]
    AnnouncementIgnored(&'static str),
    #[error("transport key error: {0}")]
    Transport(String),
    #[error("vetkd derivation failed: {0}")]
    VetKdDerivation(String),
}

pub type Result<T> = std::result::Result<T, StealthError>;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("agent error: {0}")]
    Agent(#[from] AgentError),
    #[error("candid error: {0}")]
    Candid(#[from] CandidError),
    #[error("canister error: {0}")]
    Canister(String),
}

pub type ClientResult<T> = std::result::Result<T, ClientError>;
