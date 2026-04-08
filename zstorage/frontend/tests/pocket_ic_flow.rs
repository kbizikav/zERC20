// SPDX-License-Identifier: BUSL-1.1

use candid::{CandidType, Encode};
use ic_agent::{identity::AnonymousIdentity, Agent};
use k256::ecdsa::SigningKey;
use key_manager::authorization::authorization_message;
use pocket_ic::{PocketIcBuilder, PocketIcState};
use rand::{rngs::OsRng, RngCore};
use serde::Serialize;
use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    sync::Once,
};
use storage::invoice_signature_message;
use zerc20_stealth_client::{
    authorization::{derive_address, sign_authorization, unix_time_ns},
    client::StealthCanisterClient,
    encryption::{encrypt_payload, scan_announcements},
    recipient, types,
};

#[derive(Clone, CandidType, Serialize)]
struct KeyManagerInitArgs {
    key_id_name: String,
}

#[derive(Clone, CandidType, Serialize)]
struct StorageInitArgs {
    capacity_hint: Option<u64>,
}

#[test]
fn pocket_ic_end_to_end_flow() {
    if ensure_pocket_ic_server().is_none() {
        return;
    }

    let key_manager_wasm = load_canister_wasm("zerc20_key_manager");
    let storage_wasm = load_canister_wasm("zerc20_storage");

    let mut pic = PocketIcBuilder::new()
        .with_ii_subnet() // needs for vetkey feature
        .with_state(PocketIcState::new())
        .build();

    // deploy key manager canister
    let key_manager_principal = pic.create_canister();
    pic.add_cycles(key_manager_principal, 2_000_000_000_000);
    let key_manager_init = Encode!(&KeyManagerInitArgs {
        key_id_name: "test_key_1".to_string(),
    })
    .expect("failed to encode key manager init args");
    pic.install_canister(
        key_manager_principal,
        key_manager_wasm,
        key_manager_init,
        None,
    );

    // deploy storage canister
    let storage_principal = pic.create_canister();
    pic.add_cycles(storage_principal, 2_000_000_000_000);
    let storage_init =
        Encode!(&Option::<StorageInitArgs>::None).expect("failed to encode storage init args");
    pic.install_canister(storage_principal, storage_wasm, storage_init, None);

    let rt = tokio::runtime::Runtime::new().expect("failed to create Tokio runtime");
    let replica_url = pic.make_live(None);
    let replica_url = replica_url.to_string();

    rt.block_on(async move {
        let agent = Agent::builder()
            .with_url(replica_url)
            .with_identity(AnonymousIdentity)
            .build()
            .expect("failed to build agent");
        agent
            .fetch_root_key()
            .await
            .expect("failed to fetch root key");

        let client = StealthCanisterClient::new(agent, storage_principal, key_manager_principal);

        let mut rng = OsRng;
        let signing_key = SigningKey::random(&mut rng);
        let address = derive_address(&signing_key);

        let view_public_key = client
            .get_view_public_key(address)
            .await
            .expect("failed to query view public key");

        let plaintext = b"hello from pocket-ic test";
        let encryption =
            encrypt_payload(&mut rng, &view_public_key, plaintext).expect("encryption failed");

        let announcement = client
            .submit_announcement(&encryption)
            .await
            .expect("failed to submit announcement");

        let transport = recipient::prepare_transport_key();

        // expiry in 10 minutes
        let expiry_ns = unix_time_ns()
            .expect("system time before unix epoch")
            .saturating_add(600 * 1_000_000_000);
        let max_nonce = client
            .get_max_nonce(address)
            .await
            .expect("failed to query max nonce");
        let nonce = max_nonce.saturating_add(1);
        let auth_message = authorization_message(
            key_manager_principal,
            &address,
            &transport.public,
            expiry_ns,
            nonce,
        );
        let signature = sign_authorization(&auth_message, &signing_key)
            .expect("failed to sign authorization message");

        let request = types::EncryptedViewKeyRequest {
            address: address.to_vec(),
            transport_public_key: transport.public.clone(),
            expiry_ns,
            nonce,
            signature: signature.to_vec(),
        };

        let encrypted_key = client
            .request_encrypted_view_key(&request)
            .await
            .expect("failed to request encrypted view key");

        let view_key =
            recipient::decrypt_vet_key(&encrypted_key, &view_public_key, &transport.secret)
                .expect("failed to decrypt vet key");

        let page = client
            .list_announcements(None, Some(50), types::DEFAULT_TAG)
            .await
            .expect("failed to list announcements");
        let decrypted =
            scan_announcements(&view_key, &page.announcements).expect("scan announcements");
        let recovered = decrypted
            .iter()
            .find(|entry| entry.id == announcement.id)
            .expect("announcement not decrypted");
        assert_eq!(recovered.plaintext, plaintext);
    });

    pic.stop_live();
}

#[test]
fn pocket_ic_upgrade_preserves_storage_state() {
    if ensure_pocket_ic_server().is_none() {
        return;
    }

    let key_manager_wasm = load_canister_wasm("zerc20_key_manager");
    let storage_wasm = load_canister_wasm("zerc20_storage");

    let mut pic = PocketIcBuilder::new()
        .with_ii_subnet()
        .with_state(PocketIcState::new())
        .build();

    let key_manager_principal = pic.create_canister();
    pic.add_cycles(key_manager_principal, 2_000_000_000_000);
    let key_manager_init = Encode!(&KeyManagerInitArgs {
        key_id_name: "test_key_1".to_string(),
    })
    .expect("failed to encode key manager init args");
    pic.install_canister(
        key_manager_principal,
        key_manager_wasm,
        key_manager_init,
        None,
    );

    let storage_principal = pic.create_canister();
    pic.add_cycles(storage_principal, 2_000_000_000_000);
    let storage_init =
        Encode!(&Option::<StorageInitArgs>::None).expect("failed to encode storage init args");
    pic.install_canister(storage_principal, storage_wasm.clone(), storage_init, None);

    let rt = tokio::runtime::Runtime::new().expect("failed to create Tokio runtime");
    let replica_url = pic.make_live(None);
    let replica_url = replica_url.to_string();

    let agent = Agent::builder()
        .with_url(replica_url)
        .with_identity(AnonymousIdentity)
        .build()
        .expect("failed to build agent");
    rt.block_on(async {
        agent
            .fetch_root_key()
            .await
            .expect("failed to fetch root key");
    });

    let client = StealthCanisterClient::new(agent, storage_principal, key_manager_principal);

    let mut rng = OsRng;
    let announcement = types::AnnouncementInput {
        ibe_ciphertext: random_bytes(&mut rng, 32),
        ciphertext: random_bytes(&mut rng, 64),
        nonce: random_bytes(&mut rng, 12),
        tag: types::DEFAULT_TAG.to_string(),
    };
    let signing_key = SigningKey::random(&mut rng);
    let owner = derive_address(&signing_key);
    let mut invoice_id = [0u8; 32];
    rng.fill_bytes(&mut invoice_id);
    let message = invoice_signature_message(&invoice_id, types::DEFAULT_TAG);
    let signature =
        sign_authorization(&message, &signing_key).expect("failed to sign invoice message");

    let created_id = rt.block_on(async {
        let created = client
            .submit_announcement(&announcement)
            .await
            .expect("failed to submit announcement");
        let submission = types::InvoiceSubmission {
            invoice_id: invoice_id.to_vec(),
            signature: signature.to_vec(),
            tag: types::DEFAULT_TAG.to_string(),
        };
        client
            .submit_invoice(&submission)
            .await
            .expect("failed to submit invoice");

        let before_page = client
            .list_announcements(None, Some(50), types::DEFAULT_TAG)
            .await
            .expect("failed to list announcements");
        assert!(before_page
            .announcements
            .iter()
            .any(|item| item.id == created.id));

        let before_invoices = client
            .list_invoices(owner, types::DEFAULT_TAG)
            .await
            .expect("failed to list invoices");
        assert!(before_invoices
            .iter()
            .any(|bytes| bytes.as_slice() == &invoice_id[..]));

        created.id
    });

    let upgrade_arg = Encode!(&()).expect("failed to encode upgrade args");
    pic.upgrade_canister(storage_principal, storage_wasm, upgrade_arg, None)
        .expect("failed to upgrade storage canister");

    rt.block_on(async {
        let after_page = client
            .list_announcements(None, Some(50), types::DEFAULT_TAG)
            .await
            .expect("failed to list announcements after upgrade");
        let after = after_page
            .announcements
            .iter()
            .find(|item| item.id == created_id)
            .expect("announcement missing after upgrade");
        assert_eq!(after.tag, types::DEFAULT_TAG);

        let after_invoices = client
            .list_invoices(owner, types::DEFAULT_TAG)
            .await
            .expect("failed to list invoices after upgrade");
        assert!(after_invoices
            .iter()
            .any(|bytes| bytes.as_slice() == &invoice_id[..]));
    });

    pic.stop_live();
}

fn ensure_pocket_ic_server() -> Option<PathBuf> {
    let Some(path) = std::env::var_os("POCKET_IC_BIN") else {
        eprintln!("Skipping PocketIC interaction test: pocket-ic binary not found.");
        return None;
    };

    let path = PathBuf::from(path);
    if !path.exists() {
        eprintln!(
            "Skipping PocketIC interaction test: pocket-ic binary not found at {}.",
            path.display()
        );
        return None;
    }

    if TcpListener::bind("127.0.0.1:0").is_err() {
        eprintln!(
            "Skipping PocketIC interaction test: cannot bind to loopback (network sockets disabled)."
        );
        return None;
    }

    Some(path)
}

fn random_bytes<R: RngCore>(rng: &mut R, len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; len];
    rng.fill_bytes(&mut bytes);
    bytes
}

fn load_canister_wasm(name: &str) -> Vec<u8> {
    ensure_canisters_built();
    let wasm_path = workspace_root()
        .join("target")
        .join("wasm32-unknown-unknown")
        .join("release")
        .join(format!("{name}.wasm"));
    std::fs::read(&wasm_path).unwrap_or_else(|err| {
        panic!(
            "failed to read wasm for {name} at {}: {err}",
            wasm_path.display()
        )
    })
}

fn ensure_canisters_built() {
    static BUILD_ONCE: Once = Once::new();
    BUILD_ONCE.call_once(|| {
        let status = Command::new("cargo")
            .args([
                "build",
                "--target",
                "wasm32-unknown-unknown",
                "--release",
                "-p",
                "zerc20-key-manager",
                "-p",
                "zerc20-storage",
            ])
            .current_dir(workspace_root())
            .status()
            .expect("failed to invoke cargo build for canisters");
        assert!(
            status.success(),
            "cargo build for canisters did not succeed"
        );
    });
}

fn workspace_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    for ancestor in manifest_dir.ancestors() {
        let cargo_toml = ancestor.join("Cargo.toml");
        if cargo_toml.exists() {
            if let Ok(contents) = std::fs::read_to_string(&cargo_toml) {
                if contents.contains("[workspace]") {
                    return ancestor.to_path_buf();
                }
            }
        }
    }
    manifest_dir.to_path_buf()
}
