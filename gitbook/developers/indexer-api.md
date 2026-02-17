# Indexer API Reference

The zERC20 indexer exposes a REST API for querying transfer events, generating Merkle proofs, and checking sync status.

## Base URL

| Environment | URL |
|-------------|-----|
| Public (mainnet) | Contact the zERC20 team for access |
| Self-hosted | `http://localhost:8080` (default `LISTEN_ADDR`) |

All endpoints return JSON. CORS is permissive by default.

## Endpoints

### GET /healthz

Health check.

**Response**: `200 OK` (empty body)

---

### GET /status

Returns sync status for all configured tokens.

**Response**: `200 OK`

```json
[
  {
    "label": "eth-mainnet",
    "chain_id": 1,
    "token_address": "0xEB81ab55Bc7aa89d1e0E3F60597D86e37702Af53",
    "verifier_address": "0xfb786B5E6520284Aa6a8dFA3B4F7A09ed423e25f",
    "onchain_reserved_index": 150,
    "onchain_proved_index": 148,
    "events_synced_index": 155,
    "tree_synced_index": 155,
    "ivc_generated_index": 148
  }
]
```

| Field | Type | Description |
|-------|------|-------------|
| `label` | `string` | Human-readable chain label |
| `chain_id` | `number` | EVM chain ID |
| `token_address` | `string` | zERC20 token contract address |
| `verifier_address` | `string` | Verifier contract address |
| `onchain_reserved_index` | `number?` | Latest reserved tree index on-chain |
| `onchain_proved_index` | `number?` | Latest proved tree index on-chain |
| `events_synced_index` | `number?` | Latest event index synced from chain |
| `tree_synced_index` | `number?` | Latest tree index with Merkle tree built |
| `ivc_generated_index` | `number?` | Latest tree index with IVC proof generated |

---

### GET /events

Query transfer events for a specific recipient on a specific chain.

**Query Parameters**:

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `chain_id` | `number` | Yes | EVM chain ID |
| `token_address` | `string` | Yes | zERC20 token address |
| `to` | `string` | Yes | Recipient address (burn address) |
| `limit` | `number` | No | Max results (default: 100, max: 1000) |

**Example**:

```
GET /events?chain_id=1&token_address=0xEB81...&to=0xBurn...&limit=50
```

**Response**: `200 OK`

```json
[
  {
    "event_index": 42,
    "from": "0xSender...",
    "to": "0xBurnAddress...",
    "value": "0x5f5e100",
    "eth_block_number": 24400000
  }
]
```

| Field | Type | Description |
|-------|------|-------------|
| `event_index` | `number` | Index of the event in the transfer tree |
| `from` | `string` | Sender address |
| `to` | `string` | Recipient address (burn address) |
| `value` | `string` | Transfer amount as hex-encoded uint256 |
| `eth_block_number` | `number` | Block number of the transfer |

**Errors**:
- `404` — Token not configured for the given chain and address
- `400` — Invalid limit value

---

### GET /all-events

Query transfer events for multiple recipients across all configured tokens.

**Query Parameters**:

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `recipients[]` | `string[]` | Yes | Array of recipient addresses (max 100) |
| `limit` | `number` | No | Max results per recipient (default: 100, max: 1000) |

**Example**:

```
GET /all-events?recipients[]=0xBurn1...&recipients[]=0xBurn2...&limit=20
```

**Response**: `200 OK`

```json
[
  {
    "chain_id": 1,
    "token_address": "0xEB81...",
    "event_index": 42,
    "from": "0xSender...",
    "to": "0xBurn1...",
    "value": "0x5f5e100",
    "eth_block_number": 24400000
  }
]
```

Results are sorted by `chain_id`, `token_address`, `event_index`.

**Errors**:
- `400` — More than 100 recipients

---

### POST /proofs

Generate Merkle proofs for one or more leaf indices at a given tree snapshot.

**Request Body** (`application/json`):

```json
{
  "chain_id": 1,
  "token_address": "0xEB81ab55Bc7aa89d1e0E3F60597D86e37702Af53",
  "target_index": 100,
  "leaf_indices": [42, 43, 44]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `chain_id` | `number` | Yes | EVM chain ID |
| `token_address` | `string` | Yes | zERC20 token address |
| `target_index` | `number` | Yes | Tree snapshot index to prove against |
| `leaf_indices` | `number[]` | Yes | Leaf positions to prove (max 100) |

**Response**: `200 OK`

```json
[
  {
    "target_index": 100,
    "leaf_index": 42,
    "root": "0x1a2b3c...",
    "hash_chain": "0x4d5e6f...",
    "siblings": ["0xaaa...", "0xbbb...", "..."]
  }
]
```

| Field | Type | Description |
|-------|------|-------------|
| `target_index` | `number` | Tree snapshot index |
| `leaf_index` | `number` | Position of the leaf in the tree |
| `root` | `string` | Merkle root at the target index (hex uint256) |
| `hash_chain` | `string` | Hash chain value at the target index (hex uint256) |
| `siblings` | `string[]` | Array of 40 sibling hashes for the proof path |

**Errors**:
- `404` — Token not configured, tree empty, or missing root
- `400` — Too many leaf indices (> 100) or invalid parameters

---

### GET /tree-index

Look up the tree index for a given transfer root hash.

**Query Parameters**:

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `chain_id` | `number` | Yes | EVM chain ID |
| `token_address` | `string` | Yes | zERC20 token address |
| `transfer_root` | `string` | Yes | Transfer root hash (hex uint256) |

**Example**:

```
GET /tree-index?chain_id=1&token_address=0xEB81...&transfer_root=0x1a2b3c...
```

**Response**: `200 OK`

```json
{
  "tree_index": 100
}
```

**Errors**:
- `404` — Token not configured or transfer root not found

---

## Security Notes

The indexer API is designed for **internal use**. If exposing publicly:

- Deploy behind an authentication proxy or API gateway
- Apply rate limiting to prevent abuse
- Use HTTPS termination at the load balancer
- Consider restricting CORS to your application domains
- The `/all-events` endpoint accepts up to 100 recipients; enforce additional limits as needed

## SDK Integration

The `zerc20-client-sdk` uses the indexer API internally for:

- `fetchTransferEvents()` — calls `/events` and `/all-events`
- `fetchLocalTeleportMerkleProofs()` — calls `/proofs`
- `collectRedeemContext()` — orchestrates multiple indexer calls

Pass the indexer URL via `indexerUrl` parameter in SDK functions. See the [SDK Guide](sdk/quickstart.md) for details.
