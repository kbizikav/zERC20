# Getting Started with CLI

This guide walks you through installing and using the zERC20 command-line interface.

## Installation

```bash
git clone https://github.com/kbizikav/zerc20.git
cd zerc20/cli
cargo install --path .
```

This installs the CLI binary as `zerc20-cli` (older versions may install as `cli`). Alternatively, run directly from the repository:

```bash
cd zerc20/cli
cargo run -r -- <command> ...
```

## Prerequisites

### Circuit Artifacts

Circuit artifacts are required for generating withdrawal proofs. Download them before using the CLI:

```bash
cargo install --path ../circuit-setup
zerc20-circuit-setup download --version 1.1.0
```

See [Circuit Setup](../../developers/circuit-setup.md) for more details.

## Configuration

### Environment Variables

Load environment variables from `.env` (see `.env.example` in the cli directory):

```bash
# Token configuration
export TOKENS_FILE_PATH=../config/tokens.json

# Internet Computer endpoints (required)
export IC_REPLICA_URL=<IC_URL>
export KEY_MANAGER_CANISTER_ID=<CANISTER_ID>
export STORAGE_CANISTER_ID=<CANISTER_ID>

# Required for receiving funds (path to downloaded circuit artifacts)
export NOVA_ARTIFACTS_DIR=/path/to/nova_artifacts
```

See [ICP Canister IDs](../../reference/addresses.md#icp-canister-ids) for mainnet/testnet values.

## Basic Commands

### Issue an Invoice

Generate a burn address to receive payments:

```bash
zerc20-cli invoice issue --chain-id <CHAIN_ID>
```

### List Invoices

View your invoices:

```bash
zerc20-cli invoice ls --chain-id <CHAIN_ID>
```

### Transfer

Send zERC20 to a burn address:

```bash
zerc20-cli transfer \
  --chain-id <CHAIN_ID> \
  --to <BURN_ADDRESS> \
  --amount <AMOUNT_IN_WEI>
```

### Check Invoice Status

```bash
zerc20-cli invoice status --chain-id <CHAIN_ID> --invoice-id <INVOICE_ID>
```

### Receive Funds

Generate proofs and receive:

```bash
zerc20-cli invoice receive --chain-id <CHAIN_ID> --invoice-id <INVOICE_ID>
```

## Quick Start Example

```bash
# 1. Issue an invoice
zerc20-cli invoice issue --chain-id 1

# 2. List invoices to get the invoice ID and burn address
zerc20-cli invoice ls --chain-id 1

# 3. Send funds to the burn address
zerc20-cli transfer \
  --chain-id 1 \
  --to 0x1234567890abcdef1234567890abcdef12345678 \
  --amount 1000000000000000000

# 4. Check status
zerc20-cli invoice status --chain-id 1 --invoice-id inv-01

# 5. Receive
zerc20-cli invoice receive --chain-id 1 --invoice-id inv-01
```

## Important Notes

- **Crosschain Capability**: You can send on one chain and receive on another
- **Processing Time**: Private transfers typically take 30 minutes to 1 hour on mainnet
- **Testnet Limitations**: On testnets, transfers may take longer due to LayerZero instability

## Next Steps

- [FAQ](../faq.md) — Common questions and troubleshooting
- [CLI README](https://github.com/InternetMaximalism/zerc20/tree/main/cli) — Full CLI documentation
