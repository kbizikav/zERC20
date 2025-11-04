use std::convert::TryInto;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::{CryptoRng, RngCore};

use crate::error::{Result, StealthError};
use crate::types;
use crate::types::AnnouncementInput;
use ic_vetkeys::{DerivedPublicKey, IbeCiphertext, IbeIdentity, IbeSeed, VetKey};

const SESSION_KEY_LEN: usize = 32; // 256-bit AES-GCM session key
const NONCE_LEN: usize = 12; // 96-bit nonce required by AES-GCM

pub fn encrypt_payload<R: RngCore + CryptoRng>(
    rng: &mut R,
    view_public_key_bytes: &[u8],
    plaintext: &[u8],
) -> Result<AnnouncementInput> {
    let derived_public_key = DerivedPublicKey::deserialize(view_public_key_bytes)
        .map_err(|_| StealthError::InvalidDerivedPublicKey)?;
    let identity = IbeIdentity::from_bytes(&[]);
    let seed = IbeSeed::random(rng);

    let mut session_key = [0u8; SESSION_KEY_LEN];
    rng.fill_bytes(&mut session_key);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rng.fill_bytes(&mut nonce_bytes);

    let cipher =
        Aes256Gcm::new_from_slice(&session_key).map_err(|_| StealthError::EncryptionFailed)?;
    let nonce_ga = Nonce::from(nonce_bytes);
    let ciphertext = cipher
        .encrypt(&nonce_ga, plaintext)
        .map_err(|_| StealthError::EncryptionFailed)?;
    let nonce_vec = nonce_ga.to_vec();

    let ibe_ciphertext =
        IbeCiphertext::encrypt(&derived_public_key, &identity, &session_key, &seed).serialize();

    session_key.fill(0);

    let announcement = types::AnnouncementInput {
        ibe_ciphertext,
        ciphertext,
        nonce: nonce_vec,
    };

    Ok(announcement)
}

pub fn decrypt_announcement(
    vet_key: &VetKey,
    announcement: &types::Announcement,
) -> Result<types::DecryptedAnnouncement> {
    let ibe_ciphertext = IbeCiphertext::deserialize(&announcement.ibe_ciphertext)
        .map_err(|_| StealthError::AnnouncementIgnored("invalid IBE ciphertext"))?;
    let mut session_key = match ibe_ciphertext.decrypt(vet_key) {
        Ok(key) => key,
        Err(_err) => {
            // Wrong vet key: treat as a normal miss, not an error.
            return Err(StealthError::AnnouncementIgnored("vet key mismatch"));
        }
    };
    if session_key.len() != SESSION_KEY_LEN {
        return Err(StealthError::AnnouncementIgnored(
            "unexpected session key length",
        ));
    }

    let cipher = Aes256Gcm::new_from_slice(&session_key)
        .map_err(|_| StealthError::AnnouncementIgnored("invalid AES key length"))?;
    let nonce_arr: [u8; NONCE_LEN] = announcement
        .nonce
        .as_slice()
        .try_into()
        .map_err(|_| StealthError::AnnouncementIgnored("invalid nonce length"))?;
    let nonce_ga = Nonce::from(nonce_arr);
    let plaintext = cipher
        .decrypt(&nonce_ga, announcement.ciphertext.as_ref())
        .map_err(|_| StealthError::AnnouncementIgnored("ciphertext authentication failed"))?;

    session_key.iter_mut().for_each(|b| *b = 0);

    Ok(types::DecryptedAnnouncement {
        id: announcement.id,
        plaintext,
        created_at_ns: announcement.created_at_ns,
    })
}

pub fn scan_announcements(
    vet_key: &VetKey,
    announcements: &[types::Announcement],
) -> Result<Vec<types::DecryptedAnnouncement>> {
    let mut decrypted = Vec::new();
    for announcement in announcements {
        match decrypt_announcement(vet_key, announcement) {
            Ok(message) => decrypted.push(message),
            Err(StealthError::AnnouncementIgnored(_)) => continue,
            Err(err) => return Err(err),
        };
    }
    Ok(decrypted)
}

#[cfg(test)]
mod tests {
    use rand::rngs::OsRng;

    use super::*;

    #[test]
    fn encrypt_rejects_invalid_public_key() {
        let mut rng = OsRng;
        let view_public_key = vec![0u8; 95];
        let result = encrypt_payload(&mut rng, &view_public_key, b"hello");
        assert!(matches!(result, Err(StealthError::InvalidDerivedPublicKey)));
    }
}
