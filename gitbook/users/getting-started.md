# Getting Started

This guide walks you through obtaining zERC20 tokens and making your first private transfer.

## Step 1: Get zERC20 Tokens

zERC20 tokens are ERC-20 wrapper tokens backed 1:1 by underlying assets. Choose one of these methods:

### Option A: Deposit via Frontend (Recommended)

1. Visit the [zERC20 Frontend](https://zerc20.io)
2. Connect your wallet (MetaMask, WalletConnect, etc.)
3. Select the token you want to wrap (USDC, ETH, BNB, etc.)
4. Enter the amount and confirm the deposit
5. Receive an equivalent amount of zERC20 tokens

### Option B: Deposit via CLI

```bash
# Install the CLI
cargo install zerc20-cli

# Deposit USDC to get zUSDC
zerc20 deposit --token USDC --amount 100
```

### Option C: Buy on a DEX

Purchase zERC20 directly on decentralized exchanges:
- Uniswap
- Other supported DEXes

> Check [Supported Chains](../reference/chains.md) and [Contract Addresses](../reference/addresses.md) for token addresses on each chain.

## Step 2: Make a Private Transfer

There are two patterns for private transfers:

### Pattern A: Send to a Public Address (via Frontend/CLI)

Use this when you want to send privately to someone's known address.

1. Visit the [Frontend](https://zerc20.io) or use the CLI
2. Enter the recipient's public Ethereum address
3. Enter the amount
4. The system generates a burn address, transfers your tokens, and notifies the recipient

The recipient can later withdraw the funds to any address they choose.

### Pattern B: Send to a Burn Address (Invoice Flow)

Use this when the recipient wants maximum privacy—even from the sender. This method is currently CLI-only.

1. **Recipient creates a burn address** using the CLI (see [Creating a Burn Address](#creating-a-burn-address))
2. **Recipient shares the burn address** with the sender
3. **Sender transfers zERC20** to the burn address using any standard wallet
4. **Recipient withdraws** using the Frontend or CLI

This works from **any chain**—you can send from Ethereum and the recipient can withdraw on Arbitrum.

## Creating a Burn Address

### Using the Frontend

The Frontend supports sender-generated burn addresses:

1. Go to the Send page
2. Enter the recipient's withdrawal address
3. Check the **"Pay with mobile"** option
4. A burn address will be generated for that recipient
5. Share this burn address with anyone who needs to pay the recipient

### Using the CLI (Recipient-Generated)

For maximum privacy where even the sender doesn't know the withdrawal address:

```bash
# Generate a burn address (invoice)
zerc20 invoice create --amount 100 --token zUSDC
```

The command outputs a burn address that you can share with payers.

See [Invoice Flow](cli-invoice.md) for detailed CLI instructions.

## Step 3: Receive and Withdraw

After someone sends zERC20 to your burn address:

### Using the Frontend

1. Visit the [Frontend](https://zerc20.io)
2. Connect the wallet associated with your burn address
3. View incoming transfers in the "Receive" section
4. Click "Withdraw" and choose your destination address and chain

### Using the CLI

```bash
# Scan for incoming transfers
zerc20 scan

# Withdraw to your chosen address
zerc20 withdraw --to 0xYourAddress --chain arbitrum
```

See [Receiving Funds](cli-receive.md) for detailed instructions.

## Important Notes

- **Crosschain Capability**: You can send on one chain and withdraw on another
- **Processing Time**: Cross-chain transfers may take 30 minutes to 1 hour due to LayerZero messaging
- **Testnet Limitations**: On testnets, LayerZero may occasionally experience delays beyond the typical timeframe

## Next Steps

- [Private Transfer Guide](stealth-payments.md) — Detailed privacy considerations
- [CLI Guide](cli-guide.md) — Full CLI command reference
- [Cross-chain Transfers](cross-chain.md) — How crosschain works
- [FAQ](faq.md) — Common questions and troubleshooting
