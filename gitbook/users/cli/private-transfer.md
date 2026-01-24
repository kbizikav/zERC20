# Private Transfer (CLI)

This guide explains how to send and receive private transfers using the zERC20 CLI.

## Understanding Privacy Options

The CLI supports both transfer methods:

| Method | Privacy Level | Sender Knows |
|--------|---------------|--------------|
| Sender-generated | Recipient hidden from chain | Recipient's address |
| Recipient-generated | Maximum privacy | Only burn address |

## Sending a Private Payment

### Method 1: To a Known Address (Sender-Generated)

If you know the recipient's address:

```bash
zerc20 send --to 0xRecipientAddress --amount 100 --token zUSDC
```

The CLI generates a burn address, transfers your tokens, and notifies the recipient.

### Method 2: To a Burn Address

If someone provides you with a burn address:

```bash
# Simply transfer to the burn address
zerc20 transfer --to 0xBurnAddress --amount 100 --token zUSDC
```

Or use any standard wallet (MetaMask, etc.) to send to the burn address.

## Receiving a Private Payment

### Step 1: Create an Invoice (Recipient-Generated)

Generate a burn address that you control:

```bash
zerc20 invoice create --token zUSDC --amount 100
```

Output:
```
Invoice created!
Burn Address: 0x1234...abcd
Share this address with the payer.
```

### Step 2: Share the Burn Address

Send the burn address to the payer. They can pay using any wallet.

### Step 3: Scan for Incoming Transfers

```bash
zerc20 scan
```

Output:
```
Found 1 incoming transfer:
  - 100 zUSDC from 0x1234...abcd (ready to withdraw)
```

### Step 4: Withdraw

```bash
zerc20 withdraw --to 0xYourAddress --chain ethereum
```

See [Invoice Flow](invoice.md) and [Receiving Funds](receive.md) for detailed guides.

## Protecting Amount Privacy

### Batch Withdrawals

Combine multiple incoming transfers:

```bash
zerc20 withdraw --to 0xYourAddress --batch
```

Only the total amount is exposed on-chain.

### Partial Withdrawals

Withdraw less than the full amount:

```bash
# Received 123.456789 zUSDC, withdraw only 123
zerc20 withdraw --to 0xYourAddress --amount 123
```

## Advanced Options

### Specify Chain

```bash
# Create invoice for a specific chain
zerc20 invoice create --token zUSDC --amount 100 --chain arbitrum

# Withdraw to a specific chain
zerc20 withdraw --to 0xYourAddress --chain optimism
```

### Full Example

```bash
# Create a burn address with specific parameters
zerc20 invoice create \
  --token zUSDC \
  --amount 100 \
  --chain arbitrum

# Withdraw with custom options
zerc20 withdraw \
  --to 0xNewAddress \
  --chain optimism \
  --amount 95 \
  --batch
```

## Privacy Checklist

Before making a private transfer:

- [ ] **Amount**: Is the amount generic enough? Consider rounding.
- [ ] **Timing**: Are you withdrawing immediately after receiving? Consider waiting.
- [ ] **Address reuse**: Are you withdrawing to a fresh address? Consider using a new one.
- [ ] **Batching**: Can you combine multiple transfers? Reduces fingerprinting.
- [ ] **Chain**: Are you withdrawing on a different chain? Adds another layer of privacy.
