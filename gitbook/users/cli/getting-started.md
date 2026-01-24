# Getting Started with CLI

This guide walks you through installing and using the zERC20 command-line interface.

## Installation

```bash
cargo install zerc20-cli
```

## Configuration

Set up your wallet and RPC endpoints:

```bash
# Configure your private key (or use environment variable)
export ZERC20_PRIVATE_KEY=your_private_key

# Or use a keystore file
zerc20 config --keystore /path/to/keystore.json
```

## Get zERC20 Tokens

### Option A: Deposit

Wrap underlying tokens to get zERC20:

```bash
# Deposit USDC to get zUSDC
zerc20 deposit --token USDC --amount 100

# Deposit ETH to get zETH
zerc20 deposit --token ETH --amount 1
```

### Option B: Buy on a DEX

Purchase zERC20 directly on decentralized exchanges like Uniswap.

> Check [Supported Chains](../../reference/chains.md) and [Contract Addresses](../../reference/addresses.md) for token addresses on each chain.

## Basic Commands

```bash
# Check your balance
zerc20 balance

# View pending incoming transfers
zerc20 scan

# Get help
zerc20 --help
```

## Important Notes

- **Crosschain Capability**: You can send on one chain and withdraw on another
- **Processing Time**: Private transfers typically take 30 minutes to 1 hour on mainnet
- **Testnet Limitations**: On testnets, transfers may take longer due to LayerZero instability

## Next Steps

- [Private Transfer (CLI)](private-transfer.md) — Send and receive privately
- [Invoice Flow](invoice.md) — Create invoices for receiving payments
- [Receiving Funds](receive.md) — Detailed guide for receiving transfers
- [FAQ](../faq.md) — Common questions and troubleshooting
