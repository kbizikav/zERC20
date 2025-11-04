use candid::{CandidType, Encode};
use ic_agent::{identity::AnonymousIdentity, Agent};
use k256::ecdsa::SigningKey;
use key_manager::authorization::authorization_message;
use pocket_ic::{PocketIcBuilder, PocketIcState};
use rand::rngs::OsRng;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use stealth_client::authorization::{derive_address, sign_authorization, unix_time_ns};
use stealth_client::client::StealthCanisterClient;
use stealth_client::encryption::{encrypt_payload, scan_announcements};
use stealth_client::{recipient, types};

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
        eprintln!("Skipping PocketIC interaction test: pocket-ic binary not found.");
        return;
    }

    let key_manager_wasm = load_canister_wasm("key_manager");
    let storage_wasm = load_canister_wasm("storage");

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
            .list_announcements(None, Some(50))
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

fn ensure_pocket_ic_server() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("POCKET_IC_BIN") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }
    None
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
                "key_manager",
                "-p",
                "storage",
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
