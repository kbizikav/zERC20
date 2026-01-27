# AGENTS.md - crosschain-job

Notes for AI agents working in this directory.

## Overview

Periodic maintenance worker that keeps cross-chain state in sync by:
1. Submitting `relayTransferRoot` transactions on every configured Verifier
2. Issuing `Hub.broadcast` calls for all registered LayerZero EIDs

## Directory Structure

```
crosschain-job/
├── src/
│   └── main.rs       # All logic consolidated here
├── Cargo.toml        # Package: zerc20-crosschain-job
├── .env.example      # Configuration template
└── README.md         # User-facing documentation
```

## Key Components (main.rs)

| Component | Purpose |
|-----------|---------|
| `RelayJob` | Periodically calls `Verifier.relayTransferRoot()` per chain |
| `BroadcastJob` | Periodically calls `Hub.broadcast()` to all targets |
| `HubRelayDestination` | Confirms relay delivery on Hub |
| `BroadcastDestination` | Confirms broadcast delivery on Verifiers |
| `LayerZeroProbe` | Optional LZ Scan integration for debugging stuck messages |

## Common Commands

```bash
# Run the worker (long-running)
cargo run -p zerc20-crosschain-job

# Run once and exit (smoke test)
cargo run -p zerc20-crosschain-job -- --once

# Check compilation (requires nightly)
rustup run nightly cargo check -p zerc20-crosschain-job
```

## Configuration

Key environment variables (see `.env.example`):

| Variable | Required | Description |
|----------|----------|-------------|
| `RELAY_PRIVATE_KEY` | Yes | Hex-encoded 32-byte private key |
| `TOKENS_FILE_PATH` | No | Path to tokens config (default: `../config/tokens.json`) |
| `RELAY_INTERVAL_SECS` | No | Relay loop interval (default: 300) |
| `BROADCAST_INTERVAL_SECS` | No | Broadcast loop interval (default: 600) |

## Architecture Notes

- **Single binary**: All logic is in `main.rs` (~970 lines)
- **Async runtime**: tokio multi-threaded
- **Error handling**: Uses `anyhow` with contextual errors
- **Logging**: Uses `env_logger` (set `RUST_LOG=info` or `debug`)

## Important Design Decisions

1. **Always broadcast to all targets**: Selective broadcast was removed to avoid convergence issues where agg_seq advances but only some Verifiers receive updates.

2. **Receipt timeout**: 5-minute timeout on transaction receipt to prevent jobs from hanging on RPC issues.

3. **Fee buffer**: Configurable safety margin (default +10%) on quoted LayerZero fees.

4. **Duplicate chain_id handling**: Warns but continues if Hub reports duplicate entries.

## Troubleshooting

| Symptom | Likely Cause | Solution |
|---------|--------------|----------|
| "failed to fetch transaction receipt" | RPC timeout | Check RPC health, will auto-retry next loop |
| "hub did not reflect transfer root" | LZ delivery delay | Check LZ Scan, will auto-retry |
| "no target EIDs were discovered" | Hub has no registered tokens | Register tokens on Hub contract |

## Dependencies

- `client-common`: Shared contract bindings and utilities
- `alloy`: Ethereum interaction
- `tokio`: Async runtime
- `clap`: CLI argument parsing

## Build Requirements

- **Rust nightly** (edition 2024)
- Uses let-chains and other 2024 edition features
