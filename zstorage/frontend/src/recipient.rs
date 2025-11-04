use rand::rngs::OsRng;
use rand::RngCore;

use crate::error::{Result, StealthError};
use ic_vetkeys::{DerivedPublicKey, EncryptedVetKey, TransportSecretKey, VetKey};

pub struct TransportKeyPair {
    pub secret: TransportSecretKey,
    pub public: Vec<u8>,
}

pub fn prepare_transport_key() -> TransportKeyPair {
    let mut rng = OsRng;
    let mut seed = [0u8; 32];
    rng.fill_bytes(&mut seed);
    let secret =
        TransportSecretKey::from_seed(seed.to_vec()).expect("seed length is fixed to 32 bytes");
    let public = secret.public_key();
    TransportKeyPair { secret, public }
}

pub fn decrypt_vet_key(
    encrypted_key: &[u8],
    derived_public_key: &[u8],
    transport_secret: &TransportSecretKey,
) -> Result<VetKey> {
    let encrypted =
        EncryptedVetKey::deserialize(encrypted_key).map_err(|e| StealthError::Transport(e))?;
    let derived = DerivedPublicKey::deserialize(derived_public_key)
        .map_err(|_| StealthError::Transport("invalid derived public key".into()))?;
    let vet_key = encrypted
        .decrypt_and_verify(transport_secret, &derived, &[])
        .map_err(|e| StealthError::Transport(e))?;
    Ok(vet_key)
}
