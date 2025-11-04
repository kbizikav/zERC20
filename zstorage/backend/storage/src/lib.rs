use std::cell::RefCell;
use std::collections::BTreeMap;

use alloy_primitives::{utils::eip191_message, Signature, B256};
use candid::{CandidType, Deserialize};
use ic_cdk::api::time;
use ic_cdk_macros::{init, post_upgrade, pre_upgrade, query, update};
use sha3::{Digest, Keccak256};

const MAX_CIPHERTEXT_BYTES: usize = 16 * 1024;
const MAX_IBE_CIPHERTEXT_BYTES: usize = 512;
const DEFAULT_LIST_LIMIT: u32 = 50;
const MAX_LIST_LIMIT: u32 = 200;

#[derive(Clone, CandidType, Deserialize)]
pub struct InitArgs {
    #[serde(default)]
    pub capacity_hint: Option<u64>,
}

#[derive(Clone, CandidType, Deserialize)]
pub struct AnnouncementInput {
    pub ibe_ciphertext: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
}

#[derive(Clone, CandidType, Deserialize)]
pub struct Announcement {
    pub id: u64,
    pub ibe_ciphertext: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub created_at_ns: u64,
}

#[derive(Clone, CandidType, Deserialize)]
pub struct AnnouncementPage {
    pub announcements: Vec<Announcement>,
    pub next_id: Option<u64>,
}

#[derive(Clone, CandidType, Deserialize)]
pub struct InvoiceSubmission {
    pub invoice_id: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Clone, CandidType, Deserialize, Default)]
struct State {
    announcements: Vec<Announcement>,
    next_id: u64,
    invoices: BTreeMap<[u8; 20], Vec<[u8; 32]>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

#[init]
fn init(args: Option<InitArgs>) {
    let _ = args;
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        *state = State {
            announcements: Vec::new(),
            next_id: 0,
            invoices: BTreeMap::new(),
        }
    });
}

#[pre_upgrade]
fn pre_upgrade() {
    STATE.with(|state| {
        let state = state.borrow();
        ic_cdk::storage::stable_save((state.clone(),)).expect("failed to persist storage state");
    });
}

#[post_upgrade]
fn post_upgrade() {
    let (state,): (State,) =
        ic_cdk::storage::stable_restore().expect("failed to restore storage state");
    STATE.with(|cell| {
        *cell.borrow_mut() = state;
    });
}

#[update]
fn submit_announcement(input: AnnouncementInput) -> Result<Announcement, String> {
    validate_announcement(&input)?;
    let announcement = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let announcement = Announcement {
            id: state.next_id,
            ibe_ciphertext: input.ibe_ciphertext.clone(),
            ciphertext: input.ciphertext.clone(),
            nonce: input.nonce.clone(),
            created_at_ns: time(),
        };
        state.announcements.push(announcement.clone());
        state.next_id += 1;
        announcement
    });
    Ok(announcement)
}

#[query]
fn list_announcements(start_after: Option<u64>, limit: Option<u32>) -> AnnouncementPage {
    let limit = limit.unwrap_or(DEFAULT_LIST_LIMIT).min(MAX_LIST_LIMIT) as usize;
    let (mut items, next_marker) = STATE.with(|cell| {
        let state = cell.borrow();
        let mut collected = Vec::with_capacity(limit);
        let mut next = None;
        for announcement in state.announcements.iter() {
            if let Some(start) = start_after {
                if announcement.id <= start {
                    continue;
                }
            }
            if collected.len() < limit {
                collected.push(announcement.clone());
            } else {
                next = Some(announcement.id);
                break;
            }
        }
        (collected, next)
    });

    AnnouncementPage {
        announcements: {
            items.sort_by_key(|a| a.id);
            items
        },
        next_id: next_marker,
    }
}

#[query]
fn get_announcement(id: u64) -> Option<Announcement> {
    STATE.with(|cell| {
        let state = cell.borrow();
        state.announcements.iter().find(|a| a.id == id).cloned()
    })
}

#[update]
fn submit_invoice(input: InvoiceSubmission) -> Result<(), String> {
    let invoice_id = normalize_invoice_id(&input.invoice_id)?;
    let signature = parse_signature(&input.signature)?;
    let message = invoice_signature_message(&invoice_id);
    let signer_bytes = recover_address_from_signature(&signature, &message)?;

    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let invoices = state.invoices.entry(signer_bytes).or_default();
        if !invoices.contains(&invoice_id) {
            invoices.push(invoice_id);
        }
    });

    Ok(())
}

#[query]
fn list_invoices(address: Vec<u8>) -> Result<Vec<Vec<u8>>, String> {
    let address_bytes: [u8; 20] = normalize_address(&address)?;
    let invoices = STATE.with(|cell| {
        let state = cell.borrow();
        state
            .invoices
            .get(&address_bytes)
            .map(|items| items.iter().map(|id| id.to_vec()).collect())
            .unwrap_or_default()
    });
    Ok(invoices)
}

fn validate_announcement(input: &AnnouncementInput) -> Result<(), String> {
    if input.ibe_ciphertext.is_empty() || input.ibe_ciphertext.len() > MAX_IBE_CIPHERTEXT_BYTES {
        return Err("ibe_ciphertext size is invalid".to_string());
    }
    if input.ciphertext.is_empty() || input.ciphertext.len() > MAX_CIPHERTEXT_BYTES {
        return Err("ciphertext size is invalid".to_string());
    }
    if input.nonce.len() != 12 {
        return Err("nonce must be 12 bytes (AES-GCM)".to_string());
    }
    Ok(())
}

fn normalize_invoice_id(invoice_id: &[u8]) -> Result<[u8; 32], String> {
    if invoice_id.len() != 32 {
        return Err("invoice_id must be exactly 32 bytes".to_string());
    }
    let mut normalized = [0u8; 32];
    normalized.copy_from_slice(invoice_id);
    Ok(normalized)
}

fn parse_signature(bytes: &[u8]) -> Result<Signature, String> {
    match bytes.len() {
        65 => Signature::from_raw(bytes).map_err(|err| format!("invalid signature: {err}")),
        64 => Ok(Signature::from_erc2098(bytes)),
        _ => Err("signature must be 64 or 65 bytes".to_string()),
    }
}

fn normalize_address(address: &[u8]) -> Result<[u8; 20], String> {
    if address.len() != 20 {
        return Err("address must be exactly 20 bytes".to_string());
    }
    let mut normalized = [0u8; 20];
    normalized.copy_from_slice(address);
    Ok(normalized)
}

pub fn invoice_signature_message(invoice_id: &[u8; 32]) -> Vec<u8> {
    let message = format!(
        "ICP Stealth Invoice Submission:\ninvoice_id: 0x{}",
        hex::encode(invoice_id)
    );
    eip191_message(message.as_bytes())
}

fn recover_address_from_signature(
    signature: &Signature,
    message: &[u8],
) -> Result<[u8; 20], String> {
    let digest = Keccak256::digest(message);
    let mut hash_bytes = [0u8; 32];
    hash_bytes.copy_from_slice(&digest);
    let hash = B256::from(hash_bytes);
    signature
        .recover_address_from_prehash(&hash)
        .map(|address| address.into_array())
        .map_err(|err| format!("failed to recover address: {err}"))
}

ic_cdk::export_candid!();
