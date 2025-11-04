# crosschain-job

Periodic maintenance worker that keeps cross-chain state in sync by
submitting `relayTransferRoot` transactions on every configured verifier
and issuing `Hub.broadcast` calls for all registered LayerZero EIDs.

## Prerequisites

- Rust toolchain (edition 2024) and `cargo` available in `PATH`.
- Access to RPC endpoints for every verifier chain and the hub chain, listed
  in `../config/tokens.json` or a user-provided file with the same schema.
- A funded Ethereum-compatible private key with permission to call the target
  contracts.

## Configuration

Create a copy of `.env.example` and tailor the values:

```bash
cp crosschain-job/.env.example crosschain-job/.env
```

The key variables are:

- `RELAY_PRIVATE_KEY` — required. Hex-encoded 32-byte private key used across
  every chain.
- `TOKENS_FILE_PATH` — optional. Defaults to `../config/tokens.json`. Points to
  the shared token metadata file already used by `client-common`.
- `RELAY_INTERVAL_SECS` / `BROADCAST_INTERVAL_SECS` — poll cadence per job.
- `RELAY_OPTIONS` / `BROADCAST_OPTIONS` — LayerZero options payloads encoded
  as hex. Use `0x` for empty payloads.
- `RELAY_NATIVE_FEE_BUFFER_BPS` / `BROADCAST_NATIVE_FEE_BUFFER_BPS` — fee
  safety buffers expressed in basis points (default 1000 = +10%).

Tokens are loaded through `client-common::tokens::TokensFile`, so each entry
needs an RPC URL list, verifier address, and chain identifier. The optional
`hub` section enables the broadcast job. When omitted, broadcast is
automatically disabled.

## Running

From the repository root:

```bash
cargo run -p crosschain-job
```

The binary respects CLI arguments in addition to environment overrides. Use
`--once` (or `JOB_ONCE=true`) to run the relay and broadcast logic a single
time for smoke testing:

```bash
cargo run -p crosschain-job -- --once
```

Press `Ctrl+C` to stop the long-running scheduler. The process logs every
submitted transaction and warns when hub metadata diverges from the local
configuration.
