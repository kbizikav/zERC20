# Agent Notes (Repo Root)

This repository contains both the on-chain Solidity contracts (`contracts/`) and the off-chain services (indexer, crosschain job, prover, CLI, frontend).

Use this file as a quick “how to work here safely” guide.

## High-level layout

- `contracts/`: Foundry project (deploy scripts, Solidity sources)
- `config/`: Chain/token config used by services and helper scripts (`tokens.json`, etc.)
- `cli/`: User-facing CLI
- `indexer/`, `crosschain-job/`, `decider-prover/`, `fee-manager/`: Off-chain components
- `scripts/`: Helper scripts for artifacts/config
- `config/deployed/`: Deployed contract addresses per environment (source of truth)

## Safety / secrets

- Never paste real private keys or explorer API keys into docs, commits, or chat logs.
- Prefer `.env` files that are **not committed**; use environment variables when running commands.

## “Full system” vs “contracts only”

Two common workflows exist:

1) **Full system (node + services)**
- Follow `README.md` and `cli/README.md`.
- Requires circuit artifacts + Solidity verifiers to exist in `contracts/src/verifiers/`.

2) **Contracts-only smoke testing**
- Follow `contracts/README.md`.
- Typical sequence: deploy Hub (Base) → deploy Verifier+zERC20 (Arb/OP) → deploy Liquidity+Adaptor (Arb/OP) → set peers → run `unwrapAndBridge` smoke test.

## Rust/ZK artifacts (when needed)

From repo root:

```bash
cargo run --release --bin generate_circuit_artifacts
./scripts/copy_nova_verifiers.sh
```

This is required before running the full stack or when Solidity verifier sources need to be regenerated.

## Config files

- `config/tokens.json`: primary environment description for multi-chain setups.
  - Copy from `config/tokens.example.json`.
  - Contains hub + token/verifier entries with `chain_id`, LayerZero `eid`, and RPC URLs.

## Common gotchas

- Foundry scripts and `cast` outputs sometimes include scientific notation (e.g. `10000000 [1e7]`); when exporting values,
  prefer `| awk '{print $1}'` to capture the raw integer.
- Explorer verification: many Etherscan-family explorers now require API v2. Prefer `forge verify-contract --chain-id ...`
  without forcing `--verifier-url`.

## Where to put future notes

- Contract/deployment-specific guidance: `contracts/AGENTS.md`
- Off-chain service details: prefer the component’s own README (`cli/README.md`, etc.)

