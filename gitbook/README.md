# Introduction

**zERC20** is a fully ERC-20 compliant crosschain private transfer token. Users can perform private transfers directly from standard wallets like MetaMask—no special software required. From an external observer's perspective, private transfers are indistinguishable from regular transfers.

## Key Features

- **ERC-20 Compatible**: Works with any wallet, DEX, or DeFi protocol that supports standard ERC-20 tokens
- **Private Transfers**: Send tokens without revealing the link between sender and recipient addresses
- **Crosschain Support**: Transfer privately across multiple blockchains via LayerZero
- **No Special Wallet Required**: Use MetaMask or any standard Ethereum wallet

## How It Works (In Brief)

1. A temporary "burn address" is cryptographically generated
2. The sender transfers zERC20 to this burn address (the tokens are effectively burned)
3. The recipient withdraws the same amount to any address using a zero-knowledge proof
4. The link between the burn address and the withdrawal address is never revealed on-chain

## Getting zERC20

zERC20 tokens are wrapper tokens backed 1:1 by underlying assets (USDC, ETH, BNB, etc.). You can obtain them by:

- **Depositing** the underlying token through the [Frontend](https://zerc20.io) or CLI
- **Purchasing** on decentralized exchanges like Uniswap

## Next Steps

- [How zERC20 Works](overview/how-it-works.md) — Understand the technical details
- [Getting Started](users/getting-started.md) — Start using zERC20
- [Private Transfer Guide](users/stealth-payments.md) — Step-by-step private transfer instructions
