# FAQ & Troubleshooting

## Frequently Asked Questions

### General

**What is zERC20?**

zERC20 is an ERC-20 compliant token that enables private transfers on Ethereum and other EVM chains. Unlike regular tokens where all transfers are publicly visible, zERC20 hides the link between senders and recipients using zero-knowledge proofs.

**How is zERC20 different from Tornado Cash?**

While both use zero-knowledge proofs for privacy, they differ in key ways:

| Feature | zERC20 | Tornado Cash |
|---------|--------|--------------|
| Token type | ERC-20 wrapper (zUSDC, zETH, etc.) | ETH/ERC-20 deposits |
| Fixed denominations | No — any amount | Yes — fixed pool sizes |
| Crosschain | Yes — via LayerZero | No |
| Wallet UX | Standard MetaMask transfer | Requires deposit/withdraw UI |

**What tokens are supported?**

Currently supported wrapper tokens include:
- zUSDC (wrapped USDC)
- zETH (wrapped ETH)
- zBNB (wrapped BNB)

Check the frontend for the full list of supported chains.

### Privacy

**Can anyone see my transfer amount?**

Yes, transfer and withdrawal amounts are visible on-chain. To protect amount privacy:
- Use batch withdrawals to combine multiple transfers
- Use partial withdrawals to avoid exact amount matching
- Use common/round amounts when possible

**Can the sender see where I withdraw to?**

It depends on who generated the burn address:
- **Sender-generated**: Yes, the sender knows your withdrawal address (Frontend supports this method)
- **Recipient-generated**: No, the sender only sees the burn address (CLI only)

**Is my IP address exposed?**

zERC20 itself doesn't expose IP addresses, but:
- Your RPC provider can see your IP when you submit transactions
- Consider using a VPN or Tor for additional IP privacy

### Fees and Rewards

**Why am I being charged a fee to unwrap?**

Fees are charged when unwrapping on chains with low liquidity. The fee incentivizes balanced liquidity across chains. If fees are high, consider using cross-chain unwrap to access liquidity from another chain with lower fees.

**How can I avoid high unwrap fees?**

Use **cross-chain unwrap** to unwrap via a different chain with more liquidity. The frontend shows fee comparisons for all available options.

**Why did I receive bonus tokens when wrapping?**

When a chain has low liquidity, you earn rewards for adding liquidity by wrapping. These rewards come from fees collected from previous unwraps.

**Where do I see current fees?**

Open the Wrap/Unwrap modal in the frontend and enter an amount. The interface displays the expected fee or reward before you confirm.

See [Fees and Rewards](fees-and-rewards.md) for detailed information.

### Transfers

**How long do private transfers take?**

| Network | Typical Time |
|---------|--------------|
| Mainnet | 30 minutes to 1 hour |
| Testnet | May be longer due to LayerZero instability |

**Can I cancel a transfer?**

Once zERC20 is sent to a burn address, it cannot be recovered or redirected. Only the intended recipient can withdraw it using the zero-knowledge proof.

**What happens if I send to the wrong burn address?**

The tokens are permanently locked unless the intended recipient of that burn address withdraws them. Always double-check burn addresses before sending.

---

## Troubleshooting

### My private transfer hasn't arrived

**Symptoms:** You sent zERC20 to a burn address but the recipient doesn't see it.

**Solutions:**

1. **Wait for cross-chain messaging**
   - Cross-chain transfers require 30 minutes to 1 hour for LayerZero messages to propagate
   - On testnets, this may take longer due to network instability

2. **Check transaction status**
   - Verify the original transaction was confirmed on the source chain
   - Check the burn address balance on a block explorer

3. **Check invoice status**
   - Frontend: Refresh the page
   - CLI: Run `zerc20-cli invoice status --chain-id <CHAIN_ID> --invoice-id <INVOICE_ID>`

4. **Verify burn address**
   - Confirm the sender used the correct burn address
   - EVM addresses are case-insensitive (e.g. `0xAbc...` and `0xabc...` refer to the same address), but always use the checksummed (mixed-case, EIP-55) format to catch typos

### Tokens appear stuck in a burn address

**Symptoms:** zERC20 shows in the burn address but can't be withdrawn.

**This is expected behavior.** Tokens sent to a burn address remain there permanently—they are not "moved" during withdrawal. Instead, an equivalent amount is minted to the recipient using ZKP verification.

The "stuck" tokens represent the burned supply that backs the minted withdrawal.

### REDEEM button is disabled even though transfer shows READY

**Symptoms:** Private Receive shows an announcement as READY or ARRIVED, but the REDEEM button is grayed out.

**Solutions:**

1. **Wait for proof artifacts to load**
   - The button shows "Preparing redeem resources..." while downloading proof artifacts — this is normal and may take a few seconds

2. **Reload the announcement detail**
   - If artifacts failed to load or the state is stale, click the reload button on the announcement detail
   - This clears the aggregation state cache and retries the resource download

3. **Refresh the browser**
   - If an in-app reload doesn't resolve the issue, refresh the browser page to reset all in-memory caches

### Cross-chain unwrap failed — funds stuck in Adaptor

**Symptoms:** You performed a cross-chain unwrap but the underlying token never arrived in your wallet. The transaction on your chain succeeded, but the bridging step failed partway (e.g. due to Stargate liquidity shortage).

**What happened:** Cross-chain unwrap routes your tokens through an intermediate chain for unwrapping and then bridges them back. When the bridge leg fails, your funds are held in the intermediate chain's Adaptor contract instead of being delivered to your wallet.

**Solutions:**

1. **Check for stuck funds**
   - In the frontend, navigate to the Wrap/Unwrap page — the UI will detect stuck funds automatically
   - SDK users can call `hasStuckFunds()` or `fetchAdaptorBalances()` to check programmatically

2. **Withdraw stuck funds**
   - Use the frontend's recovery UI to withdraw back to your wallet
   - SDK users can call `withdrawFromAdaptor()` to rescue the funds

3. **Retry the operation**
   - After recovering your funds, you can retry the cross-chain unwrap or use a local unwrap instead

> **Note:** Stuck funds are safe — they remain in the Adaptor contract and can be recovered at any time by the original user.

---

## Still Need Help?

- Check the [CLI Guide](cli/getting-started.md) for command reference
- Review [How zERC20 Works](../overview/how-it-works.md) for technical details
- Join our [Discord](https://discord.gg/5HCgDyW7tD) for community support
- Contact us on Telegram: [@zERC20bot](https://t.me/zERC20bot)
- Email us at **help@zerc20.io**

For full support details, see the [Support](../reference/support.md) page.
