use alloy_primitives::keccak256;
use anyhow::{anyhow, Context, Result};
use k256::ecdsa::SigningKey;
use std::time::{SystemTime, UNIX_EPOCH};

/// Derive the Ethereum-style address (last 20 bytes of Keccak-256 hash) from a signing key.
pub fn derive_address(signing_key: &SigningKey) -> [u8; 20] {
    let verifying_key = signing_key.verifying_key();
    let encoded = verifying_key.to_encoded_point(false);
    let public_key = encoded.as_bytes();
    let digest: [u8; 32] = keccak256(&public_key[1..]).into();

    let mut address = [0u8; 20];
    address.copy_from_slice(&digest[12..]);
    address
}

/// Sign an authorization message and return the r||s||v signature bytes (v in {27,28}).
pub fn sign_authorization(message: &[u8], signing_key: &SigningKey) -> Result<[u8; 65]> {
    let digest: [u8; 32] = keccak256(message).into();

    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(&digest)
        .map_err(|err| anyhow!("failed to sign authorization message: {err}"))?;

    let mut bytes = [0u8; 65];
    bytes[..64].copy_from_slice(&signature.to_bytes());
    bytes[64] = recovery_id.to_byte().saturating_add(27);
    Ok(bytes)
}

/// Return the current Unix timestamp in nanoseconds.
pub fn unix_time_ns() -> Result<u64> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before Unix epoch")?;
    Ok(now
        .as_secs()
        .saturating_mul(1_000_000_000)
        .saturating_add(now.subsec_nanos() as u64))
}
