# ICP Storage Specification

## Overview

The Internet Computer (ICP) provides the stealth messaging layer for zERC20. Two canisters handle encrypted communication between senders and recipients without revealing the relationship on-chain.

**Location**: `zstorage/`

## Components

### Key Manager Canister

**Location**: `zstorage/backend/key_manager/`

Derives identity-based encryption (IBE) keys using VetKD (Verifiable Encrypted Threshold Key Derivation).

**Functions**:
- Derives Boneh-Franklin IBE secrets per EVM address
- Enforces nonce + TTL on key requests
- Recipients authenticate with EVM signatures to fetch view keys

### Storage Canister

**Location**: `zstorage/backend/storage/`

Persists encrypted announcements and signed invoices.

**Functions**:
- Store/retrieve encrypted announcements
- Store/retrieve signed invoices
- Paginated scanning for recipients

## Data Structures

### Invoice

Recipient-initiated payment request.

```
Invoice {
    invoice_id: String,
    signer: Address,           // EVM address that signed
    burn_addresses: Vec<Address>,
    mode: InvoiceMode,         // Single or Batch
    created_at: Timestamp,
}
```

### Announcement

Sender-initiated encrypted payload.

```
Announcement {
    announcement_id: String,
    recipient: Address,        // EVM address
    ibe_ciphertext: Bytes,     // IBE-encrypted AES key
    aes_payload: Bytes,        // AES-GCM encrypted stealth payload
    created_at: Timestamp,
}
```

### Stealth Payload

The decrypted content of an announcement.

```
StealthPayload {
    chain_id: u256,
    recipient_address: Address,
    tweak: u256,
    secret: u256,
    burn_address: Address,
}
```

## Workflows

### Invoice Flow (Recipient-Initiated)

```
1. Recipient signs invoice request with EVM wallet
2. Recipient calls storage.submit_invoice()
3. Storage persists invoice with derived burn addresses
4. Recipient shares burn address(es) with payer
5. Payer sends zERC20 to burn address
6. Recipient later redeems via ZKP
```

**Modes**:
- **Single**: One burn address per invoice
- **Batch**: Up to 10 burn addresses (sub IDs 0-9)

### Payment Advice Flow (Sender-Initiated)

```
1. Sender derives (tweak, secret) from their seed
2. Sender computes burn address for recipient
3. Sender fetches recipient's IBE public key from key manager
4. Sender encrypts stealth payload:
   - Generate random AES key
   - Encrypt payload with AES-GCM
   - Encrypt AES key with IBE
5. Sender calls storage.submit_announcement()
6. Sender transfers zERC20 to burn address
7. Recipient scans announcements, decrypts, and redeems
```

### Recipient Scanning

```
1. Recipient authenticates with key manager (EVM signature + transport key)
2. Key manager returns encrypted view key
3. Recipient decrypts view key
4. Recipient fetches announcements from storage (paginated)
5. For each announcement:
   - Decrypt IBE ciphertext to get AES key
   - Decrypt AES payload to get stealth payload
   - Check if burn address has funds via indexer
6. Recipient stores matched payloads locally
```

## Encryption Scheme

### IBE (Identity-Based Encryption)

- **Scheme**: Boneh-Franklin IBE
- **Identity**: Recipient's EVM address
- **Key Derivation**: VetKD from ICP subnet keys

### Payload Encryption

```
1. Generate random 256-bit AES key
2. Encrypt stealth payload with AES-256-GCM
3. Encrypt AES key with recipient's IBE public key
4. Store (IBE ciphertext, AES ciphertext) as announcement
```

### Decryption

```
1. Request encrypted view key from key manager (authenticated)
2. Decrypt view key with transport private key
3. Use view key to decrypt IBE ciphertext → AES key
4. Use AES key to decrypt payload → stealth payload
```

## Client Libraries

### Rust Client

**Location**: `zstorage/frontend/` (Rust)

```rust
use zstorage::StealthCanisterClient;

let client = StealthCanisterClient::new(ic_url, key_manager_id, storage_id);

// Issue invoice
client.submit_invoice(chain_id, mode, signature).await?;

// Publish announcement
client.submit_announcement(recipient, encrypted_payload).await?;

// Scan for incoming
let announcements = client.scan_announcements(my_address, page).await?;
```

### TypeScript Client

**Location**: `frontend/src/services/sdk/storage/`

Browser-compatible client with same functionality.

## Security Considerations

- **Key Manager Trust**: ICP subnet collectively holds master key; no single node can decrypt
- **Storage Privacy**: Canisters store only encrypted data; cannot read contents
- **Authentication**: EVM signatures required for key requests
- **Nonce/TTL**: Prevents replay attacks on key derivation requests
