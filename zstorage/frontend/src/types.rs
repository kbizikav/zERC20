use candid::CandidType;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, CandidType)]
pub struct AnnouncementInput {
    pub ibe_ciphertext: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, CandidType)]
pub struct Announcement {
    pub id: u64,
    pub ibe_ciphertext: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub created_at_ns: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, CandidType)]
pub struct AnnouncementPage {
    pub announcements: Vec<Announcement>,
    pub next_id: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, CandidType)]
pub struct InvoiceSubmission {
    pub invoice_id: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, CandidType)]
pub struct DecryptedAnnouncement {
    pub id: u64,
    pub plaintext: Vec<u8>,
    pub created_at_ns: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, CandidType)]
pub struct EncryptedViewKeyRequest {
    pub address: Vec<u8>,
    pub transport_public_key: Vec<u8>,
    pub expiry_ns: u64,
    pub nonce: u64,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, CandidType)]
pub struct EncryptedViewKeyResponse {
    pub encrypted_key: Vec<u8>,
    pub view_public_key: Vec<u8>,
}
