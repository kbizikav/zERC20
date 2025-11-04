use alloy_primitives::{utils::eip191_message, Signature as AlloySignature, B256};
use candid::Principal;
use sha3::{Digest, Keccak256};
use thiserror::Error;

pub type Address = [u8; 20];

#[derive(Debug, Error)]
pub enum AuthorizationError {
    #[error("signature must be 65 bytes (r||s||v)")]
    InvalidLength,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("failed to recover signer")]
    RecoveryFailed,
}

pub fn authorization_message(
    canister_id: Principal,
    address: &Address,
    transport_public_key: &[u8],
    expiry_ns: u64,
    nonce: u64,
) -> Vec<u8> {
    let message = format!(
        "ICP Stealth Authorization:\naddress: 0x{}\ncanister: {}\ntransport: 0x{}\nexpiry_ns:{expiry_ns}\nnonce:{nonce}",
        hex::encode(address),
        canister_id.to_text(),
        hex::encode(transport_public_key),
    );
    eip191_message(message.as_bytes())
}

pub fn recover_address_from_signature(
    message: &[u8],
    signature: &[u8],
) -> Result<Address, AuthorizationError> {
    let signature =
        AlloySignature::from_raw(signature).map_err(|_| AuthorizationError::InvalidSignature)?;
    let digest = Keccak256::digest(message);
    let mut hash_bytes = [0u8; 32];
    hash_bytes.copy_from_slice(&digest);
    let hash = B256::from(hash_bytes);
    let address = signature
        .recover_address_from_prehash(&hash)
        .map_err(|_| AuthorizationError::RecoveryFailed)?;
    Ok(address.into_array())
}
