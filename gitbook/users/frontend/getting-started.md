# Getting Started with Frontend

This guide walks you through using the zERC20 web application.

## Step 1: Access the Frontend

Visit the [zERC20 Frontend](https://app.zerc20.io/).

> **Testing?** Use the [Testnet Frontend](https://v2.testnet.app.zerc20.io/) for testing with test tokens.

<figure><img src="../../assets/frontend_how_to/dashboard-overview.png" alt="Dashboard Overview" width="560"><figcaption>Dashboard Overview</figcaption></figure>

## Step 2: Connect Your Wallet

1. Click "Connect Wallet" in the top right corner
2. Select your wallet provider (MetaMask, WalletConnect, etc.)
3. Approve the connection request in your wallet

Once connected, your wallet address and token balances will be displayed in the dashboard.

## Step 3: Get zERC20 Tokens

zERC20 tokens are ERC-20 wrapper tokens backed 1:1 by underlying assets. These tokens enable private transfers while maintaining full compatibility with the ERC-20 standard.

You can select the token type and chain from the dropdowns at the top of the page:

| Token Selector | Chain Selector |
|:--------------:|:--------------:|
| <img src="../../assets/frontend_how_to/token-selector.png" alt="Token Selector" width="200"> | <img src="../../assets/frontend_how_to/chain-selector.png" alt="Chain Selector" width="200"> |

### Option A: Wrap Tokens

Wrapping converts your standard tokens (USDC, ETH, etc.) into zERC20 tokens at a 1:1 ratio.

1. Click the "Wrap / Unwrap" button
2. Ensure the "WRAP" tab is selected
3. Select the token you want to wrap (USDC, ETH, BNB, etc.)
4. Enter the amount to wrap

<figure><img src="../../assets/frontend_how_to/wrap-modal-input.png" alt="Wrap Modal with Amount" width="480"><figcaption>Wrap Modal - Enter amount to convert</figcaption></figure>

5. Click "Wrap USDC to zUSDC" (or the appropriate token)
6. Confirm the transaction in your wallet
7. Receive an equivalent amount of zERC20 tokens

> **Wrap Rewards**: If the chain has low liquidity, you may receive bonus tokens as a reward for adding liquidity. See [Fees and Rewards](../fees-and-rewards.md) for details.

After wrapping, your balance will be updated:

<figure><img src="../../assets/frontend_how_to/dashboard-after-wrap.png" alt="Dashboard After Wrap" width="560"><figcaption>Dashboard showing updated zERC20 balance</figcaption></figure>

### Option B: Buy on a DEX

Purchase zERC20 directly on decentralized exchanges like Uniswap.

> Check [Contract Addresses](../../reference/addresses.md) for token addresses on each chain.

### Unwrapping zERC20 Tokens

To convert zERC20 tokens back to the underlying asset:

1. Click the "Wrap / Unwrap" button
2. Select the "UNWRAP" tab
3. Enter the amount to unwrap
4. Set your preferred slippage tolerance (0.5%, 3%, 10%, or Custom)
5. Select the destination chain from the "UNWRAP ON" dropdown

**Same Chain Unwrap:**

Select your current chain (shown as "Chain (Current)") to receive the underlying tokens on the same chain.

<figure><img src="../../assets/frontend_how_to/unwrap-same-chain.png" alt="Unwrap Same Chain" width="480"><figcaption>Unwrap to current chain</figcaption></figure>

**Cross-Chain Unwrap:**

Select a different chain from the "UNWRAP ON" dropdown to access liquidity on another chain. The process works as follows:

1. Your zERC20 tokens are bridged from your current chain (Chain A) to the destination chain (Chain B) via LayerZero
2. On Chain B, the tokens are unwrapped to the underlying asset (e.g., USDC)
3. The underlying tokens are bridged back to your current chain (Chain A)

For example, if you're on Arbitrum and select "Base" as the unwrap destination, the flow is: **Arbitrum → Base → Arbitrum**. You receive the underlying tokens on Arbitrum, but the unwrap happens on Base.

<figure><img src="../../assets/frontend_how_to/unwrap-cross-chain.png" alt="Unwrap Cross Chain" width="480"><figcaption>Cross-chain unwrap using liquidity from another network</figcaption></figure>

The fee breakdown (unwrap fee, bridge fee, LayerZero fee) is shown before confirming the transaction.

> **Fee Optimization**: If unwrap fees are high on your current chain due to low liquidity, cross-chain unwrap lets you access liquidity from another chain with lower fees. The frontend shows fee comparisons so you can choose the best option. See [Fees and Rewards](../fees-and-rewards.md) for details.

### Unwrap History

After executing a cross-chain unwrap, you can track its progress in the **History** tab of the Wrap / Unwrap dialog.

| Status | Meaning |
|--------|---------|
| **In Transit** | Tokens are being bridged via LayerZero / Stargate |
| **Finalizing** | Tokens have arrived on the destination but the status is still updating |
| **Completed** | Unwrap is fully complete |

The History tab auto-refreshes when you reopen it. After a cross-chain unwrap, the frontend polls for the LayerZero Scan index update automatically, showing "Waiting for LayerZero Scan to index your latest transfer..." until the entry appears.

> **Tip:** If the history looks stale after switching chains, the tab automatically clears and re-fetches data for the newly selected chain.

### Recovering Stuck Funds

In rare cases, a cross-chain unwrap may fail partway through (e.g. due to temporary Stargate liquidity shortage). When this happens, your funds remain safely in the destination chain's Adaptor contract.

The frontend detects stuck funds automatically and displays an amber **Recoverable Balances in Adaptor** banner at the top of the History tab. To recover:

1. Open the **Wrap / Unwrap** dialog and go to the **History** tab
2. If the stuck funds are on a different chain, click **"Switch to \<chain\>"** to switch networks first
3. Click the **Withdraw** button next to each stuck balance
4. Confirm the transaction in your wallet
5. The recovered funds are returned to your wallet

> **Note:** Stuck funds are safe — they remain in the Adaptor contract and can be recovered at any time.

## Step 4: Make a Private Transfer

See [Private Transfer Guide](private-transfer.md) for detailed instructions on sending.

See [Scan Receives Guide](scan-receives.md) for instructions on receiving transfers.

## Important Notes

- **Crosschain Capability**: You can send on one chain and withdraw on another using LayerZero messaging
- **Processing Time**: Private transfers typically take 30 minutes to 1 hour on mainnet
- **Testnet Limitations**: On testnets, transfers may take longer due to LayerZero instability

## Next Steps

- [Private Transfer (Frontend)](private-transfer.md) — Send and receive privately
- [FAQ](../faq.md) — Common questions and troubleshooting
