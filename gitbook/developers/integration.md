# Integration Guide

This guide covers how to integrate zERC20 into your application.

## Overview

zERC20 can be integrated at multiple levels:

1. **Token Integration**: Use zERC20 as a standard ERC-20 token
2. **Private Transfers**: Enable stealth payments in your app
3. **Full Stack**: Run your own indexer for maximum privacy

## Token Integration

zERC20 is fully ERC-20 compatible. Any application that supports ERC-20 tokens can use zERC20.

### Contract Addresses

See [Contract Addresses](../reference/addresses.md) for deployment addresses on each chain.

### Standard ERC-20 Interface

```solidity
interface IERC20 {
    function balanceOf(address account) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
    function approve(address spender, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
}
```

## Private Transfer Integration

To enable private transfers in your application:

### 1. Generate Burn Address

Use the client libraries to generate burn addresses for recipients.

**Rust**:
```rust
use client_common::payment::{compute_burn_address, find_pow_nonce};

let (secret, nonce) = find_pow_nonce(chain_id, recipient_address, tweak)?;
let burn_address = compute_burn_address(chain_id, recipient_address, tweak, secret)?;
```

**TypeScript**:
```typescript
import { computeBurnAddress, findPowNonce } from '@zerc20/sdk';

const { secret, nonce } = await findPowNonce(chainId, recipientAddress, tweak);
const burnAddress = computeBurnAddress(chainId, recipientAddress, tweak, secret);
```

### 2. Transfer to Burn Address

Standard ERC-20 transfer to the burn address:

```solidity
IERC20(zERC20).transfer(burnAddress, amount);
```

### 3. Publish Announcement (Optional)

If using sender-initiated flow, publish encrypted payload to ICP storage:

```typescript
import { StealthClient } from '@zerc20/sdk';

const client = new StealthClient(icpUrl, keyManagerId, storageId);
await client.publishAnnouncement(recipientAddress, encryptedPayload);
```

### 4. Recipient Withdrawal

Recipients use CLI or Frontend to scan for transfers and generate ZK proofs.

## API Integration

### Indexer API

The indexer provides HTTP endpoints for querying transfers and Merkle proofs.

**Base URL**: Configured per deployment

**Endpoints**:

```
GET /transfers?address={burnAddress}
  → Returns transfers to a burn address

GET /merkle-proof?index={leafIndex}
  → Returns Merkle proof for a leaf

GET /status
  → Returns indexer sync status
```

### Decider Prover API

For batch withdrawals, proofs must be finalized by the decider prover.

```
POST /prove
  Content-Type: application/json
  {
    "circuit_kind": "WithdrawLocal",
    "ivc_proof": "<base64>"
  }
  → Returns finalized proof
```

## SDK Libraries

### Rust SDK

**Crate**: `client-common`

```rust
use client_common::{
    payment::{FullBurnAddress, compute_burn_address},
    indexer::IndexerClient,
    decider::DeciderClient,
};
```

### TypeScript SDK

**Package**: `@zerc20/sdk`

```typescript
import {
  computeBurnAddress,
  IndexerClient,
  StealthClient,
} from '@zerc20/sdk';
```

## Running Your Own Indexer

For maximum privacy, run your own indexer instance to avoid sender-recipient linkage leaks.

### Docker Compose

```bash
git clone https://github.com/kbizikav/zERC20.git
cd zERC20/docker
docker-compose up -d
```

### Configuration

Set environment variables:

```bash
DATABASE_URL=postgres://...
RPC_URL=https://...
TOKENS_FILE_PATH=./config/tokens.json
```

See [Deployment Guide](deployment/README.md) for full setup instructions.

## Security Considerations

### Indexer Privacy

The hosted indexer can observe:
- Sender address
- Burn address
- Transfer value
- Recipient address (when they query)

**Mitigation**: Run your own indexer or use Tor/VPN when querying.

### Amount Fingerprinting

Unique amounts can link deposits and withdrawals.

**Mitigation**: Use round amounts, batch withdrawals, or partial withdrawals.

### Timing Analysis

Immediate withdrawals after deposits can be correlated.

**Mitigation**: Introduce delays between deposit and withdrawal.
