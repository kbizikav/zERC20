# Private Transfer (Frontend)

This guide explains how to send and receive private transfers using the zERC20 web application.

## Understanding Privacy

Before making a private transfer, understand what the Frontend supports:

- **Sender-generated burn addresses**: The sender needs to know the recipient's withdrawal address
- The recipient's address is hidden from on-chain observers, but known to the sender

For maximum privacy where even the sender doesn't know the withdrawal address, use the [CLI](../cli/private-transfer.md).

## Sending a Private Payment

### To a Known Address

If you know the recipient's address:

1. Visit the [Frontend](https://v2.testnet.app.zerc20.io/)
2. Navigate to the Send page
3. Enter the recipient's Ethereum address
4. Enter the amount of zERC20 to send
5. Confirm the transaction in your wallet

The system generates a burn address, transfers your tokens, and notifies the recipient.

### To a Burn Address

If someone provides you with a burn address:

1. Open MetaMask (or any wallet)
2. Send zERC20 to the provided burn address
3. Done — the recipient handles the withdrawal

You can send from any supported chain.

## Creating a Burn Address for Others

To let someone pay you (or pay on behalf of a recipient):

1. Go to the Send page
2. Enter the recipient's withdrawal address
3. Check the **"Pay with mobile"** option
4. A burn address will be generated for that recipient
5. Share this burn address with anyone who needs to pay

> Note: With this method, you (the burn address creator) will know the recipient's withdrawal address.

## Receiving and Withdrawing

After someone sends zERC20 to your burn address:

1. Visit the [Frontend](https://v2.testnet.app.zerc20.io/)
2. Connect the wallet associated with your burn address
3. View incoming transfers in the "Receive" section
4. Click "Withdraw" and choose:
   - Destination chain
   - Withdrawal address (can be any address you control)
5. Confirm the withdrawal

The frontend automatically scans for incoming transfers when you connect your wallet.

## Protecting Amount Privacy

Since amounts are visible on-chain, consider these strategies:

### Batch Withdrawals

Combine multiple incoming transfers into a single withdrawal:

1. In the Receive section, select multiple pending transfers
2. Click "Withdraw" to combine them
3. Only the total amount is exposed on-chain

### Partial Withdrawals

Withdraw less than the full amount to obscure the original transfer amount.

## Privacy Checklist

Before making a private transfer:

- [ ] **Amount**: Is the amount generic enough? Consider rounding.
- [ ] **Timing**: Are you withdrawing immediately after receiving? Consider waiting.
- [ ] **Address reuse**: Are you withdrawing to a fresh address? Consider using a new one.
- [ ] **Batching**: Can you combine multiple transfers? Reduces fingerprinting.
- [ ] **Chain**: Are you withdrawing on a different chain? Adds another layer of privacy.
