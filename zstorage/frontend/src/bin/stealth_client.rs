use anyhow::{Context, Result};
use candid::Principal;
use clap::Parser;
use ic_agent::{identity::AnonymousIdentity, Agent};
use k256::ecdsa::SigningKey;
use key_manager::authorization::authorization_message;
use rand::{rngs::OsRng, RngCore};
use stealth_client::{
    authorization::{derive_address, sign_authorization, unix_time_ns},
    client::StealthCanisterClient,
    encryption::{encrypt_payload, scan_announcements},
    recipient, types,
};

#[derive(Parser)]
#[command(
    name = "stealth-client",
    about = "Run an end-to-end stealth announcement flow against deployed canisters"
)]
struct Cli {
    #[arg(
        long,
        default_value = "http://127.0.0.1:4943",
        help = "Replica base URL (e.g. http://127.0.0.1:4943 or https://ic0.app)"
    )]
    replica_url: String,
    #[arg(long, help = "Key manager canister principal (text format)")]
    key_manager: String,
    #[arg(long, help = "Storage canister principal (text format)")]
    storage: String,
    #[arg(
        long,
        default_value = "Hello from the Rust stealth client!",
        help = "Plaintext message to encrypt and announce"
    )]
    message: String,
    #[arg(
        long,
        default_value_t = 600,
        help = "Authorization TTL in seconds for request_encrypted_view_key"
    )]
    authorization_ttl_seconds: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let agent = build_agent(&cli).await?;
    let storage_id =
        Principal::from_text(&cli.storage).context("failed to parse storage canister principal")?;
    let key_manager_id = Principal::from_text(&cli.key_manager)
        .context("failed to parse key manager canister principal")?;
    let client = StealthCanisterClient::new(agent, storage_id, key_manager_id);

    run_demo_flow(&cli, &client).await?;
    Ok(())
}

async fn build_agent(cli: &Cli) -> Result<Agent> {
    let agent = Agent::builder()
        .with_url(cli.replica_url.clone())
        .with_identity(AnonymousIdentity)
        .build()
        .context("failed to build agent")?;

    if is_local_replica(&cli.replica_url) {
        agent
            .fetch_root_key()
            .await
            .context("failed to fetch replica root key")?;
    }

    Ok(agent)
}

async fn run_demo_flow(cli: &Cli, client: &StealthCanisterClient) -> Result<()> {
    println!("Replica URL: {}", cli.replica_url);
    println!(
        "Key manager canister: {}\nStorage canister: {}",
        client.key_manager_canister_id(),
        client.storage_canister_id()
    );

    let mut rng = OsRng;
    let signing_key = SigningKey::random(&mut rng);
    let address = derive_address(&signing_key);
    println!("Using ephemeral sender address: 0x{}", hex::encode(address));

    let view_public_key = client
        .get_view_public_key(address)
        .await
        .context("failed to query view public key")?;
    println!("View public key: 0x{}", hex::encode(&view_public_key));

    let plaintext = cli.message.as_bytes();
    let encryption =
        encrypt_payload(&mut rng, &view_public_key, plaintext).context("encryption failed")?;

    let announcement = client
        .submit_announcement(&encryption)
        .await
        .context("failed to submit announcement")?;
    println!(
        "Submitted announcement id {} (ciphertext {} bytes)",
        announcement.id,
        announcement.ciphertext.len()
    );

    let transport = recipient::prepare_transport_key();
    let now_ns = unix_time_ns()?;
    let expiry_ns =
        now_ns.saturating_add(cli.authorization_ttl_seconds.saturating_mul(1_000_000_000));
    let nonce = rng.next_u64();

    let auth_message = authorization_message(
        client.key_manager_canister_id(),
        &address,
        &transport.public,
        expiry_ns,
        nonce,
    );
    let signature = sign_authorization(&auth_message, &signing_key)?;
    println!("Authorization signature: 0x{}", hex::encode(signature));

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
        .context("failed to request encrypted view key")?;
    println!("Encrypted vet key: 0x{}", hex::encode(&encrypted_key));

    let view_key = recipient::decrypt_vet_key(&encrypted_key, &view_public_key, &transport.secret)
        .context("failed to decrypt vet key response")?;
    println!(
        "Recovered VetKey signature bytes: 0x{}",
        hex::encode(view_key.serialize())
    );

    let page = client
        .list_announcements(None, Some(50))
        .await
        .context("failed to list announcements")?;
    let decrypted = scan_announcements(&view_key, &page.announcements)
        .context("failed to decrypt announcements")?;

    if let Some(found) = decrypted.iter().find(|entry| entry.id == announcement.id) {
        println!(
            "Decrypted announcement {} -> {}",
            found.id,
            String::from_utf8_lossy(&found.plaintext)
        );
    } else {
        println!(
            "Warning: submitted announcement id {} was not decrypted",
            announcement.id
        );
    }

    Ok(())
}

fn is_local_replica(url: &str) -> bool {
    url.contains("127.0.0.1") || url.contains("localhost")
}
