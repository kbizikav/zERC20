# AGENTS.md - indexer

Notes for AI agents working in this directory.

## Overview

The tree indexer is a multi-job service that:
1. Indexes `IndexedTransfer` events from blockchain to PostgreSQL
2. Builds incremental Merkle trees from indexed events
3. Compiles IVC proofs and submits proved transfer roots on-chain
4. Provides HTTP API for tree state queries and Merkle proof generation

## Directory Structure

```
indexer/
├── src/
│   ├── main.rs           # Entry point, job orchestration
│   ├── lib.rs            # Crate module exports
│   ├── config.rs         # Configuration loading and validation
│   ├── error.rs          # Error type aliases
│   ├── events/
│   │   └── mod.rs        # Event indexing from blockchain (~780 lines)
│   ├── jobs/
│   │   ├── mod.rs        # Job module exports
│   │   ├── event.rs      # Event sync job loop
│   │   ├── tree.rs       # Tree ingestion job loop
│   │   ├── root.rs       # Root prover job (~1240 lines)
│   │   ├── lock.rs       # Distributed lease-based locking
│   │   └── utils.rs      # Type conversion helpers
│   ├── trees/
│   │   └── db.rs         # PostgreSQL-backed Merkle tree (~900 lines)
│   └── server/
│       └── mod.rs        # HTTP API server (actix-web)
├── tests/                # Integration tests
├── Cargo.toml            # Package: zerc20-tree-indexer
├── .env.example          # Configuration template
└── README.md             # User documentation
```

## Key Components

| Component | File | Purpose |
|-----------|------|---------|
| `EventIndexer` | events/mod.rs | Pulls events from RPC, stores in DB with adaptive block span |
| `EventSyncJob` | jobs/event.rs | Periodic event sync loop with lease-based locking |
| `TreeIngestionJob` | jobs/tree.rs | Converts events to Merkle tree leaves |
| `RootProverJob` | jobs/root.rs | IVC proof compilation + on-chain submission |
| `DbIncrementalMerkleTree` | trees/db.rs | PostgreSQL-backed Merkle tree with history |
| `LeaseGuard` | jobs/lock.rs | Distributed lock for multi-instance coordination |

## Common Commands

```bash
# Run the indexer (requires IS_SYNC=true for background jobs)
IS_SYNC=true cargo run -p zerc20-tree-indexer

# Run once and exit (smoke test)
IS_SYNC=true cargo run -p zerc20-tree-indexer -- --once

# HTTP server only (no background jobs)
cargo run -p zerc20-tree-indexer

# Check compilation (requires nightly)
rustup run nightly cargo check -p zerc20-tree-indexer

# Run tests (requires PostgreSQL + Anvil)
cargo test -p zerc20-tree-indexer
```

## Important Design Decisions

1. **Lease-based locking**: Jobs use PostgreSQL advisory locks with TTL to coordinate across multiple instances. Leases auto-expire after 30 seconds if not renewed.

2. **Startup lease cleanup**: On startup, expired leases are cleaned up to handle ungraceful shutdowns (SIGKILL, panics). See `cleanup_expired_leases()`.

3. **Adaptive block span**: Event indexer automatically reduces block span when RPC providers reject large range queries. Detection uses pattern matching on error messages.

4. **History window**: Tree snapshots are retained for `TREE_HISTORY_WINDOW` indices to support Merkle proof generation for past states. Must be >= (current - latest_proved).

5. **Chain ID validation at config time**: Token chain_id is validated during job construction, not at runtime, to fail fast on invalid configuration.

## Error Handling Patterns

- **No `.expect()` on fallible operations**: Use `ok_or_else()`, `context()`, or safe defaults with comments
- **Custom error types**: `EventIndexerError`, `DbMerkleTreeError` with structured variants
- **Graceful degradation**: Unrecognized block range errors are logged as warnings for operator visibility

## Database Schema

Key tables (managed by sqlx migrations):
- `tokens` - Registered token metadata
- `indexed_transfer_events` - Partitioned event storage
- `event_indexer_state` - Per-token sync progress
- `merkle_nodes_current` - Current tree node values
- `merkle_node_updates` - Historical node changes
- `merkle_snapshots` - Tree state snapshots
- `leases` - Distributed lock state

## Dependencies

- `zkp`: Zero-knowledge proof primitives (Poseidon hash, IVC)
- `client-common`: Contract bindings, token config parsing
- `api-types`: Shared API type definitions
- `alloy`: Ethereum interaction
- `sqlx`: PostgreSQL async driver
- `actix-web`: HTTP server framework
- `tokio`: Async runtime

## Build Requirements

- **Rust nightly** (edition 2024)
- **PostgreSQL** for runtime and tests
- **Anvil** (optional) for integration tests

## Troubleshooting

| Symptom | Likely Cause | Solution |
|---------|--------------|----------|
| Jobs not starting | `IS_SYNC` not set | Set `IS_SYNC=true` |
| "provider rejected block range" | RPC limit | Auto-handled; check logs for span reduction |
| "lease was not held" | Lock contention | Normal with multiple instances |
| Proof generation fails | History window too small | Increase `TREE_HISTORY_WINDOW` |
| "LeaseGuard dropped without release" | Ungraceful shutdown | Informational; cleanup happens on next startup |
