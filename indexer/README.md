# Tree Indexer

`tree-indexer` runs two coordinated jobs in a single Tokio runtime:

- **Event sync job** – pulls `IndexedTransfer` events for every configured token and stores
  them in Postgres using the existing indexer logic.
- **Tree ingestion job** – watches for newly indexed, contiguous events and appends them into
  the partitioned Merkle tree tables.

Run both jobs together via:

```bash
cargo run -p tree-indexer -- --tokens ../config/tokens.json
```

Use `--once` to execute a single iteration of each job (helpful in cron or test scripts).

## Database Setup

Make sure the Postgres database defined by `DATABASE_URL` exists and has the latest schema before starting the indexer:

```bash
sqlx database setup
```

This creates the database if needed and runs all pending migrations.

## Configuration Inputs

### Token Metadata (`tokens.json`)

Provide optional hub metadata and token definitions in JSON (default path `../config/tokens.json` or `TOKENS_FILE_PATH` env). The `hub` block is ignored by the indexer if omitted:

```json
{
  "hub": {
    "hub_address": "0x0000000000000000000000000000000000000001",
    "chain_id": 5,
    "rpc_urls": [
      "https://eth-goerli.g.alchemy.com/v2/YOUR_KEY"
    ]
  },
  "tokens": [
    {
      "label": "goerli-test",
      "token_address": "0x1111111111111111111111111111111111111111",
      "verifier_address": "0x2222222222222222222222222222222222222222",
      "chain_id": 5,
      "deployed_block_number": 12345678,
      "rpc_urls": [
        "https://eth-goerli.g.alchemy.com/v2/YOUR_KEY",
        "https://goerli.infura.io/v3/YOUR_PROJECT_ID"
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

Each token must include at least one RPC URL (string or array). Duplicate labels or addresses are allowed but will map to distinct advisory locks.

### Environment Variables

Set the runtime parameters through environment variables (see `.env.example`):

- `DATABASE_URL` – Postgres connection string
- `EVENT_INTERVAL_MS` – poll frequency for event sync (default `5000`)
- `EVENT_BLOCK_SPAN` – block span per RPC batch (default `5000`)
- `EVENT_FORWARD_SCAN_OVERLAP` – overlap blocks to catch reorg gaps (default `10`)
- `TREE_INTERVAL_MS` – poll frequency for tree ingestion (default `2000`)
- `TREE_HEIGHT` – Merkle tree height (default `64`)
- `TREE_HISTORY_WINDOW` – retained history window for proofs (default `100`)
- `TREE_BATCH_SIZE` – leaf append batch size (default `128`)

Use `.env` during development or pass variables directly when invoking the binary.
