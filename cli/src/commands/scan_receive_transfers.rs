use std::{collections::HashSet, fs, path::PathBuf};

use alloy::primitives::{Address, B256};
use anyhow::{Context, Result, anyhow};
use client_common::payment::burn_address::FullBurnAddress;
use client_common::tokens::TokenEntry;
use hex;
use k256::{FieldBytes, ecdsa::SigningKey};
use key_manager::authorization::authorization_message;
use serde::{Deserialize, Serialize};
use stealth_client::{
    authorization::{derive_address, sign_authorization, unix_time_ns},
    encryption::scan_announcements,
    recipient::{decrypt_vet_key, prepare_transport_key},
    types::EncryptedViewKeyRequest,
};

use crate::{CommonArgs, ScanReceiveTransfersArgs, commands::shared::build_stealth_client};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScannedTransfer {
    pub id: u64,
    pub recipient_chain_id: u64,
    pub recipient_address: String,
    pub burn_address: String,
    pub full_burn_address_hex: String,
    pub created_at_ns: u64,
}

pub async fn run(
    common: &CommonArgs,
    args: &ScanReceiveTransfersArgs,
    _tokens: &[TokenEntry],
    private_key: B256,
) -> Result<()> {
    let client = build_stealth_client(common)
        .await
        .context("failed to construct stealth canister client")?;

    let signing_key = signing_key_from_b256(private_key)?;
    let recipient_address = address_from_signing_key(&signing_key);
    let mut address_bytes = [0u8; 20];
    address_bytes.copy_from_slice(recipient_address.as_slice());

    println!("Scanning announcements for {}", recipient_address);

    let view_public_key = client
        .get_view_public_key(address_bytes)
        .await
        .context("failed to query view public key")?;

    let transport = prepare_transport_key();
    let expiry_ns = unix_time_ns()
        .context("failed to fetch current unix time")?
        .saturating_add(args.authorization_ttl_seconds.saturating_mul(1_000_000_000));
    let max_nonce = client
        .get_max_nonce(address_bytes)
        .await
        .context("failed to query max nonce")?;
    let nonce = max_nonce.saturating_add(1);

    let auth_message = authorization_message(
        client.key_manager_canister_id(),
        &address_bytes,
        &transport.public,
        expiry_ns,
        nonce,
    );
    let signature = sign_authorization(&auth_message, &signing_key)
        .context("failed to sign authorization request")?;

    let request = EncryptedViewKeyRequest {
        address: address_bytes.to_vec(),
        transport_public_key: transport.public.clone(),
        expiry_ns,
        nonce,
        signature: signature.to_vec(),
    };

    let encrypted_key = client
        .request_encrypted_view_key(&request)
        .await
        .context("failed to request encrypted view key")?;
    let vet_key = decrypt_vet_key(&encrypted_key, &view_public_key, &transport.secret)
        .context("failed to decrypt vet key response")?;

    let mut start_after = None;
    let mut saved = Vec::new();
    loop {
        let page = client
            .list_announcements(start_after, Some(args.page_size))
            .await
            .context("failed to list announcements")?;

        if page.announcements.is_empty() {
            break;
        }

        let decrypted = scan_announcements(&vet_key, &page.announcements)
            .context("failed to decrypt announcements")?;
        for entry in decrypted {
            let burn_payload = match FullBurnAddress::from_bytes(&entry.plaintext) {
                Ok(burn) => burn,
                Err(err) => {
                    eprintln!(
                        "Warning: announcement {} could not decode FullBurnAddress: {err}",
                        entry.id
                    );
                    continue;
                }
            };

            if !is_recipient(&burn_payload, recipient_address) {
                continue;
            }

            let burn_address = match burn_payload.burn_address() {
                Ok(address) => address,
                Err(err) => {
                    eprintln!(
                        "Warning: announcement {} could not derive burn address: {err}",
                        entry.id
                    );
                    continue;
                }
            };
            println!("Announcement {} -> {}", entry.id, burn_address);

            saved.push(ScannedTransfer {
                id: entry.id,
                recipient_chain_id: burn_payload.gr.chain_id,
                recipient_address: recipient_address_string(&burn_payload),
                burn_address: burn_address.to_string(),
                full_burn_address_hex: format!("0x{}", hex::encode(&entry.plaintext)),
                created_at_ns: entry.created_at_ns,
            });
        }

        start_after = page
            .announcements
            .last()
            .map(|announcement| announcement.id);
        if page.next_id.is_none() {
            break;
        }
    }

    write_results(&args.output, &saved).context("failed to persist scanned transfers")?;
    println!(
        "Captured {} transfer(s); results written to {}",
        saved.len(),
        args.output.display()
    );

    Ok(())
}

fn signing_key_from_b256(secret: B256) -> Result<SigningKey> {
    let mut raw = [0u8; 32];
    raw.copy_from_slice(secret.as_slice());
    let field_bytes: FieldBytes = raw.into();
    SigningKey::from_bytes(&field_bytes)
        .map_err(|_| anyhow!("failed to derive signing key from PRIVATE_KEY"))
}

fn address_from_signing_key(signing_key: &SigningKey) -> Address {
    let derived = derive_address(signing_key);
    Address::from_slice(&derived)
}

fn is_recipient(burn: &FullBurnAddress, recipient: Address) -> bool {
    recipient_address_from_payload(burn) == recipient
}

fn recipient_address_from_payload(burn: &FullBurnAddress) -> Address {
    Address::from_word(burn.gr.address)
}

fn recipient_address_string(burn: &FullBurnAddress) -> String {
    recipient_address_from_payload(burn).to_string()
}

fn write_results(path: &PathBuf, transfers: &[ScannedTransfer]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
    }
    let mut existing: Vec<ScannedTransfer> = if path.exists() {
        let metadata = fs::metadata(path)
            .with_context(|| format!("failed to access metadata for {}", path.display()))?;
        if metadata.len() == 0 {
            Vec::new()
        } else {
            let file = fs::File::open(path)
                .with_context(|| format!("failed to open {} for reading", path.display()))?;
            serde_json::from_reader(file).with_context(|| {
                format!(
                    "failed to parse existing scanned transfers in {}",
                    path.display()
                )
            })?
        }
    } else {
        Vec::new()
    };

    let mut seen_ids: HashSet<u64> = existing.iter().map(|entry| entry.id).collect();
    let mut modified = false;
    for transfer in transfers {
        if seen_ids.insert(transfer.id) {
            existing.push(transfer.clone());
            modified = true;
        }
    }

    if !path.exists() && transfers.is_empty() {
        modified = true; // ensure the file is created even if no transfers found
    }

    if modified {
        let file = fs::File::create(path)
            .with_context(|| format!("failed to open {} for writing", path.display()))?;
        serde_json::to_writer_pretty(file, &existing)
            .context("failed to serialize scanned announcements")?
    }

    Ok(())
}
