# ICP Stealth Announcement Protocol Specification

## 1. Objectives

- Deliver stealth payment announcements on the Internet Computer Protocol (ICP) without forcing recipients to pre-register public keys.
- Leverage vetKD (verifiably encrypted threshold key derivation) so the key manager canister derives per-recipient IBE private keys from a shared master secret while recipients alone recover the secret material.
- Let recipients detect announcements addressed to them by attempting identity-based decryption with keys derived through vetKD.

## 2. Actors and Components

- **Sender** - Encrypts a payload for the recipient and publishes the announcement to the storage canister.
- **Recipient** - Controls an EVM-compatible address, registers a transport key, and scans announcements by attempting IBE decryption per record.
- **Key manager canister**
  - Stores vetKD configuration (key identifier, domain separator, optional master public key bytes).
  - Derives per-recipient IBE private keys through management canister vetKD calls without persisting recipient secrets.
  - Issues encrypted private keys to authenticated recipients using recipient supplied transport keys.
- **Storage canister** - Persists announcement records (IBE ciphertext wrapper, AES-GCM ciphertext, nonce) and exposes paginated query APIs.
- **Client libraries (Rust / TypeScript)** - Provide high-level helpers for senders and recipients, including key derivation, encryption, and scanning logic.

## 3. Cryptographic Foundations

### 3.1 vetKD Master Configuration

- The key manager is provisioned with `VetKDKeyId { curve: bls12_381_g2, name: "key_1" }` on mainnet (or `"test_key_1"` / `"dfx_test_key"` on development networks) and a domain separator `context = b"icp-stealth-announcement-v1"`.
- The corresponding master public key bytes may be stored in configuration for clients that need to verify IBE computations offline, but canister derivations rely exclusively on vetKD system calls.
- Inputs to `vetkd_derive_key` currently use an empty vector (`input = []`); the derivation is fully bound by the context field.

### 3.2 Identity Hash Derivation (Boneh-Franklin IBE)

1. `get_view_public_key` invokes the management canister vetKD public key API. The request uses the recipient address bytes in the context so the derived point aligns with the identity used by both senders and recipients.
   ```rust
   use ic_cdk::management_canister::{vetkd_public_key, VetKDPublicKeyArgs};

   async fn vetkd_public_key_for(address: [u8; 20]) -> Vec<u8> {
       let request = VetKDPublicKeyArgs {
           canister_id: None,
           context: context(address), // domain separator || address bytes
           key_id: bls12_381_g2_test_key(),
       };

       let reply = vetkd_public_key(&request)
           .await
           .expect("failed to derive vetKD identity point");
       reply.public_key
   }
   ```
   _Note:_ the same derivation can be reproduced off-chain with the published vetKD algorithms; the system call simply delegates the canonical computation to the IC.
2. The key manager never caches the result. It recomputes `q_id` on each query to avoid storing recipient state.
3. Recipients who later decrypt a vetKD response can verify correctness by pairing `q_id` with the recovered private key `d_id` and confirming it matches the published master public key parameters.

### 3.3 Sender IBE Encryption Workflow

1. Call `get_view_public_key` on the key manager to obtain the derived public key bytes (`DerivedPublicKey`) bound to the recipient address.
2. Generate a fresh 256-bit session key `k_enc` and a 96-bit AES-GCM nonce.
3. Encrypt the payload with AES-GCM-256 using `(k_enc, nonce)` to produce the announcement ciphertext.
4. Build an `ic_vetkeys::IbeIdentity` from the recipient address, sample an `IbeSeed::random`, and invoke `IbeCiphertext::encrypt` with the derived public key, identity, session key, and seed. Serialize the result to produce `ibe_ciphertext`.
5. Submit the announcement `{ ibe_ciphertext, ciphertext, nonce }` to the storage canister.

### 3.4 Recipient Decryption Workflow

1. Generate a fresh transport secret via `ic_vetkeys::TransportSecretKey::random()` and expose the public component to the key manager.
2. Sign the authorization message (see §4.2) with the wallet that controls the 20-byte address, then call `key_manager.request_encrypted_view_key`.
3. The key manager validates expiry, nonce, and signature, then calls `ic_cdk::management_canister::vetkd_derive_key` with:
   - `context = context(address_bytes)` (domain separator concatenated with the recipient address).
   - `input = []` (empty vector).
   - `key_id = bls12_381_g2_test_key()` (or `"key_1"` in production).
   - `transport_public_key` supplied by the recipient.
   The management canister returns `encrypted_key`, which only the recipient can decrypt with the matching transport secret.
4. After decrypting the vetKD payload, the recipient obtains the VetKey for the address. For each announcement, the client deserializes `ibe_ciphertext`, decrypts it with the VetKey to recover `k_enc`, and attempts AES-GCM decryption with `(k_enc, nonce)`. Authentication success confirms the announcement is addressed to the recipient; failures imply the message targets someone else.

## 4. Canister Interfaces

### 4.1 Key Manager (`key_manager/src/lib.rs`)

State:
- vetKD configuration (`key_id`, `context`, optional `master_public_key` bytes).
- Replay protection map keyed by `(address, nonce)` with expiry timestamps.

Public methods:

| Method | Type | Description |
| --- | --- | --- |
| `get_master_public_key() -> Vec<u8>` | query | Returns the configured vetKD master public key for offline verification. |
| `get_view_public_key(address: [u8; 20]) -> ViewPublicKeyResponse` | query | Calls `vetkd_public_key` and returns the hashed identity point plus optional master public key bytes for senders. |
| `get_max_nonce(address: [u8; 20]) -> u64` | query | Returns the highest nonce previously accepted for the given address, or `0` if the address has never authorized. |
| `request_encrypted_view_key(req: EncryptedViewKeyRequest) -> EncryptedViewKeyResponse` | update | Verifies the wallet signature, enforces nonce/expiry, invokes `vetkd_derive_key`, and returns the encrypted IBE private key bound to the recipient’s transport key. |

Authorization message (EIP-191 style):
```
ICP Stealth IBE Key Request
Address: 0x<recipient_hex>
TransportPub: <base64>
Expiry: <ISO8601 UTC>
Nonce: <u64>
```

### 4.2 Storage (`storage/src/lib.rs`)

State:
- Vector of `Announcement` records with an optional capacity cap.
- Derived metrics (e.g., counts per ciphertext size bucket) if needed.

Record layout:
```
struct Announcement {
    ibe_ciphertext: Vec<u8>,  // Serialized ic_vetkeys::IbeCiphertext
    ciphertext: Vec<u8>,      // AES-GCM ciphertext
    nonce: [u8; 12],
    created_at_ns: u64,
}
```

Public methods:

| Method | Type | Description |
| --- | --- | --- |
| `announce(payload: AnnouncementInput)` | update | Validates sizes, appends the announcement, and performs eviction if the capacity limit is reached. |
| `list_recent(limit: u32, cursor: Option<u64>) -> Vec<Announcement>` | query | Returns the newest announcements up to a limit (default cap of 1,000). |
| `latest_index() -> u64` | query | Exposes the most recent announcement index for incremental polling. |

Business rules:
- Enforce strict payload size limits to control cycles and storage usage.
- Track hashes of `(ibe_ciphertext, nonce)` pairs to detect trivial replay within a recent window.
- Consider rate limiting or deposits for spam mitigation in future iterations.

## 5. Client Libraries

### 5.1 Rust Crate (`stealth_client`)

- `types` - Rust structs mirroring the candid interface exported by the canisters.
- `StealthCanisterClient` - Thin wrapper around the candid interfaces for key manager and storage canisters.
- `encrypt_payload` - Uses the derived public key from `get_view_public_key` to produce an IBE-wrapped session key plus AES-GCM ciphertext.
- `recipient` - Manages transport keys, prepares signed requests, decrypts vetKD replies, and scans announcements by attempting IBE decryption per ciphertext.
- `sender` - Provides helpers for authorization message construction and address recovery.
- Testing - Integration tests with PocketIC validate the end-to-end IBE + AES flow.

### 5.2 TypeScript Package (`ts-client`)

- `config.ts` - Loads canister IDs and vetKD settings for Node.js and browser environments.
- `sender.ts` - Fetches encryption parameters through `@dfinity/agent`, runs the IBE encryption primitives (WASM or pure JS fallback), and returns announcement payloads.
- `recipient.ts` - Wraps vetKD transport key generation, authorization signing helpers, vetKD decryption, and storage scanning logic that attempts IBE decryption per record.
- `types.ts` - Type definitions generated from candid using `didc` and re-exported for consumers.
- Testing - Browser and Node.js suites that exercise HKDF determinism, vetKD round-trips, and PocketIC-backed integration flows.

## 6. Local Testing Workflows

### 6.1 `dfx` Replica

1. Start a clean replica: `dfx start --background --clean --host 127.0.0.1:4943`.
2. Deploy the key manager and storage canisters; configure `dfx_test_key` (or another development key ID) in the key manager settings.
3. Use `dfx canister call` commands to invoke `get_view_public_key` and `request_encrypted_view_key`, retrieving the identity point and encrypted IBE private key while iterating on the client libraries.

### 6.2 PocketIC Automation

1. Use the `pocket-ic` crate in Rust integration tests to spin up an in-process replica, deploy both canisters, and assert sender/recipient flows without running an external `dfx` process.
2. Reuse the same binary in TypeScript integration tests by targeting the PocketIC REST API and keeping WASM bundles pre-built for offline execution.

## 7. Data Formats and Serialization

- All canister interfaces use candid; both Rust and TypeScript code generation rely on `ic_cdk::export_candid!`.
- Binary values (keys, ciphertexts, nonces) are encoded as `Vec<u8>` / `Uint8Array`. Helper utilities expose hex and base64 representations for user interfaces.
- Timestamps use nanoseconds since Unix epoch (`u64`) sourced from `ic_cdk::api::time()`.

## 8. Security Considerations

- **Transport key confidentiality** - Recipients must generate new transport key pairs per request; only the encrypted IBE private key leaves the management canister.
- **Replay protection** - Signed requests include nonces and expiries enforced by the key manager to block replayed authorizations.
- **Cycle budgeting** - Each `vetkd_derive_key` call incurs management canister cycles; controllers must provision adequate cycles for high-volume use.
- **Ciphertext probing** - Recipients only confirm ownership after AES-GCM authentication succeeds; failed decryptions reveal no information about other recipients beyond the need to attempt decryption.
- **Payload hygiene** - Hard limits on ciphertext and IBE wrapper sizes help prevent denial-of-service via oversized submissions.
