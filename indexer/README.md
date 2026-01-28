# Tree Indexer

`zerc20-tree-indexer` runs three coordinated jobs and an HTTP server in a single Tokio runtime:

- **Event sync job** – Pulls `IndexedTransfer` events for every configured token and stores them in PostgreSQL.
- **Tree ingestion job** – Watches for newly indexed, contiguous events and appends them into the partitioned Merkle tree tables.
- **Root prover job** – Compiles IVC proofs for processed events and submits proved transfer roots to the verifier contract.
- **HTTP server** – Provides REST API endpoints for querying tree state and generating Merkle proofs.

## Quick Start

```bash
# Set up environment
cp indexer/.env.example indexer/.env
# Edit .env with your DATABASE_URL and other settings

# Run database migrations
sqlx database setup

# Start the indexer with all jobs enabled
IS_SYNC=true cargo run -p zerc20-tree-indexer
```

Use `--once` to execute a single iteration of each job (helpful for testing or cron scripts):

```bash
IS_SYNC=true cargo run -p zerc20-tree-indexer -- --once
```

## Database Setup

Ensure the PostgreSQL database defined by `DATABASE_URL` exists and has the latest schema:

```bash
sqlx database setup
```

This creates the database if needed and runs all pending migrations.

## Configuration

### Token Metadata (`tokens.json`)

Provide token definitions in JSON (default path `../config/tokens.json` or `TOKENS_FILE_PATH` env):

```json
{
  "hub": {
    "hub_address": "0x0000000000000000000000000000000000000001",
    "chain_id": 11155111,
    "rpc_urls": ["https://eth-sepolia.g.alchemy.com/v2/YOUR_KEY"]
  },
  "tokens": [
    {
      "label": "sepolia-test",
      "token_address": "0x1111111111111111111111111111111111111111",
      "verifier_address": "0x2222222222222222222222222222222222222222",
      "chain_id": 11155111,
      "deployed_block_number": 12345678,
      "rpc_urls": [
        "https://eth-sepolia.g.alchemy.com/v2/YOUR_KEY",
        "https://sepolia.infura.io/v3/YOUR_PROJECT_ID"
      ]
    },
    {
      "label": "anvil-local",
      "token_address": "0x3333333333333333333333333333333333333333",
      "verifier_address": "0x4444444444444444444444444444444444444444",
      "chain_id": 31337,
      "deployed_block_number": 0,
      "rpc_urls": "http://127.0.0.1:8545"
    }
  ]
}
```

Each token must include at least one RPC URL (string or array). The `hub` block is optional and only used by the crosschain-job.

### Environment Variables

See `.env.example` for the complete list with detailed descriptions. Key variables:

#### Job Control
| Variable | Default | Description |
|----------|---------|-------------|
| `IS_SYNC` | `false` | Set to `true` to enable background sync jobs |
| `LISTEN_ADDR` | `localhost:8080` | HTTP server bind address |

#### Event Indexer
| Variable | Default | Description |
|----------|---------|-------------|
| `EVENT_INTERVAL_MS` | `5000` | Poll frequency for event sync |
| `EVENT_BLOCK_SPAN` | `5000` | Block span per RPC batch |
| `EVENT_FORWARD_SCAN_OVERLAP` | `10` | Overlap blocks for reorg protection |

#### Tree Ingestion
| Variable | Default | Description |
|----------|---------|-------------|
| `TREE_INTERVAL_MS` | `2000` | Poll frequency for tree ingestion |
| `TREE_HEIGHT` | `64` | Merkle tree height (must match verifier) |
| `TREE_HISTORY_WINDOW` | `100` | Retained snapshots for proof generation |
| `TREE_BATCH_SIZE` | `128` | Events per database transaction |

#### Root Prover
| Variable | Default | Description |
|----------|---------|-------------|
| `ROOT_INTERVAL_MS` | `5000` | Poll frequency for IVC compilation |
| `ROOT_SUBMIT_INTERVAL_MS` | `10000` | Poll frequency for proof submission |
| `DECIDER_PROVER_URL` | `http://127.0.0.1:8081` | Decider prover service URL |
| `DECIDER_PROVER_TIMEOUT_SECS` | `120` | Timeout for Groth16 proof generation |
| `ROOT_SUBMITTER_PRIVATE_KEY` | - | Private key for on-chain submissions |

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     zerc20-tree-indexer                     │
├─────────────────┬─────────────────┬─────────────────────────┤
│  Event Sync Job │ Tree Ingestion  │     Root Prover Job     │
│                 │      Job        │                         │
│  RPC ──► Events │ Events ──► Tree │ Tree ──► IVC ──► Submit │
│         (DB)    │         (DB)    │              (On-chain) │
└─────────────────┴─────────────────┴─────────────────────────┘
                            │
                    ┌───────┴───────┐
                    │  HTTP Server  │
                    │  (REST API)   │
                    └───────────────┘
```

## HTTP API

When the server is running, the following endpoints are available:

- `GET /health` – Health check
- `GET /tokens` – List configured tokens
- `GET /tokens/{label}/state` – Get tree state for a token
- `GET /tokens/{label}/proof?leaf_index={n}&target_index={m}` – Generate Merkle proof

## Troubleshooting

| Symptom | Likely Cause | Solution |
|---------|--------------|----------|
| Jobs not running | `IS_SYNC` not set | Set `IS_SYNC=true` |
| "provider rejected block range" | RPC rate limit | Reduce `EVENT_BLOCK_SPAN` |
| "tree state ahead of events" | Database inconsistency | Check event indexer logs |
| Proof generation fails | History window too small | Increase `TREE_HISTORY_WINDOW` |
