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

**Is zERC20 compliant with regulations?**

zERC20 is a privacy tool. Users are responsible for ensuring their use complies with applicable laws in their jurisdiction.

**What tokens are supported?**

Currently supported wrapper tokens include:
- zUSDC (wrapped USDC)
- zETH (wrapped ETH)
- zBNB (wrapped BNB)

Check [Supported Chains](../reference/chains.md) for the full list.

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

### Transfers

**How long do private transfers take?**

| Transfer Type | Typical Time |
|---------------|--------------|
| Same chain | ~5 minutes |
| Cross-chain | 30 minutes to 1 hour |
| Testnet (cross-chain) | May be longer due to LayerZero instability |

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

3. **Rescan for transfers**
   - Frontend: Refresh the page or click "Scan"
   - CLI: Run `zerc20 scan --force`

4. **Verify burn address**
   - Confirm the sender used the correct burn address
   - Burn addresses are case-sensitive

### Tokens appear stuck in a burn address

**Symptoms:** zERC20 shows in the burn address but can't be withdrawn.

**This is expected behavior.** Tokens sent to a burn address remain there permanently—they are not "moved" during withdrawal. Instead, an equivalent amount is minted to the recipient using ZKP verification.

The "stuck" tokens represent the burned supply that backs the minted withdrawal.

### Withdrawal transaction fails

**Symptoms:** The withdrawal transaction reverts or fails.

**Solutions:**

1. **Check proof generation**
   - Ensure the ZKP was generated correctly
   - Try regenerating the proof: CLI `zerc20 withdraw --regenerate-proof`

2. **Verify chain state**
   - The Merkle root may not be synced yet
   - Wait 10-15 minutes and retry

3. **Check gas settings**
   - ZKP verification requires significant gas (~500k-1M)
   - Ensure sufficient gas limit

4. **Cross-chain root sync**
   - For cross-chain withdrawals, the global root must be synced
   - Check if the aggregation sequence is current

### Frontend not loading transfers

**Symptoms:** The Frontend shows no pending transfers despite confirmed deposits.

**Solutions:**

1. **Clear browser cache**
   - Cached state may be stale
   - Try incognito/private browsing mode

2. **Check wallet connection**
   - Ensure the correct wallet is connected
   - Try disconnecting and reconnecting

3. **Indexer sync**
   - The indexer may be catching up with recent blocks
   - Wait a few minutes and refresh

### CLI command errors

**"Insufficient funds" error**

Ensure you have:
- Enough zERC20 for the transfer
- Enough native token (ETH, BNB, etc.) for gas

**"Proof generation failed" error**

- Check that the burn address was generated correctly
- Verify you're using the correct secret/tweak values
- Try with `--verbose` flag for detailed logs

**"Network error" error**

- Check your internet connection
- Verify the RPC endpoint is responsive
- Try a different RPC provider

---

## Still Need Help?

- Check the [CLI Guide](cli-guide.md) for command reference
- Review [How zERC20 Works](../overview/how-it-works.md) for technical details
- Open an issue on [GitHub](https://github.com/piatoss3612/zERC20/issues)
