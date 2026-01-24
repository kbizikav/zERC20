# Private Transfer Guide

This guide provides detailed instructions for making private transfers with zERC20, including privacy best practices.

## Understanding Privacy Levels

Before making a private transfer, understand what information remains private and what becomes public.

### What Is Public

| Information | Visibility |
|-------------|------------|
| Transaction to burn address | Visible on-chain |
| Transfer amount | Visible on-chain |
| Withdrawal amount | Visible on-chain |
| Burn address | Visible on-chain |

### What Is Private

| Information | Privacy Level |
|-------------|---------------|
| Link between sender and recipient | Hidden on-chain |
| Recipient's withdrawal address | Hidden on-chain |
| Which burn address maps to which withdrawal | Hidden via ZKP |

## Choosing a Transfer Method

### Method 1: Sender-Generated Burn Address

**Privacy**: The recipient's address is hidden from on-chain observers but known to the sender.

**Flow:**
1. Recipient shares their withdrawal address with the sender (off-chain)
2. Sender generates a burn address derived from the recipient's address via Frontend or CLI
3. Sender transfers zERC20 to the burn address
4. Recipient withdraws using the ZKP

**Best for:** Sending to someone whose address you already know.

### Method 2: Recipient-Generated Burn Address

**Privacy**: Maximum — the sender does not know the recipient's withdrawal address.

**Flow:**
1. Recipient generates a burn address via CLI
2. Recipient shares only the burn address with the sender
3. Sender transfers zERC20 to the burn address
4. Recipient withdraws to any address they choose

**Best for:** Receiving payments, donations, or any case where you don't want the payer to know your final address.

> Note: The Frontend currently supports Method 1 (sender-generated). The CLI supports both methods.

## Protecting Amount Privacy

Since amounts are visible on-chain, unusual amounts can serve as "fingerprints" that link transactions.

### Strategy 1: Batch Withdrawals

Combine multiple incoming transfers into a single withdrawal:

```
Received:
  - 50 zUSDC from Burn Address A
  - 30 zUSDC from Burn Address B
  - 20 zUSDC from Burn Address C

Withdraw:
  - 100 zUSDC (combined) → Only total visible on-chain
```

**How to batch:**
- **Frontend**: Select multiple pending transfers and withdraw together
- **CLI**: `zerc20 withdraw --batch`

### Strategy 2: Partial Withdrawals

Withdraw less than the full amount to obscure the original transfer:

```
Received: 123.456789 zUSDC

Withdraw: 123 zUSDC (round number)
Remaining: 0.456789 zUSDC (can withdraw later or leave)
```

This breaks the amount correlation between the incoming and outgoing transactions.

## Step-by-Step: Receiving a Private Payment

### 1. Create an Invoice (Burn Address)

**Frontend (sender-generated):**

The sender needs to know your withdrawal address. They will:
1. Navigate to Send page
2. Enter your withdrawal address
3. Enable "Pay with mobile"
4. The system generates a burn address tied to your address

**CLI (recipient-generated, maximum privacy):**

You generate the burn address yourself, so the sender never learns your withdrawal address:
```bash
zerc20 invoice create --token zUSDC --amount 100
```

### 2. Share the Burn Address

Send the burn address to the payer through any channel:
- Messaging app
- Email
- QR code
- Payment link

The burn address looks like a normal Ethereum address, so the payer can send from any wallet.

### 3. Wait for Payment

The payer sends zERC20 to the burn address using MetaMask or any wallet. The transaction is a standard ERC-20 transfer.

### 4. Scan for Incoming Transfers

**Frontend:** Automatically scans when you connect your wallet.

**CLI:**
```bash
zerc20 scan
```

### 5. Withdraw

Once the transfer is detected and the proof is ready:

**Frontend:**
1. Select the pending transfer
2. Choose destination chain
3. Enter withdrawal address (can be any address you control)
4. Confirm withdrawal

**CLI:**
```bash
zerc20 withdraw --to 0xYourAddress --chain ethereum
```

## Step-by-Step: Sending a Private Payment

### Method 1: To a Burn Address (Payer's View)

If someone provides you with a burn address:

1. Open MetaMask (or any wallet)
2. Send zERC20 to the provided burn address
3. Done — the recipient handles the withdrawal

You can send from any supported chain.

### Method 2: Via Frontend

If you know the recipient's address and want to send privately:

1. Visit the [Frontend](https://zerc20.io)
2. Enter the recipient's address
3. Enter the amount
4. Confirm the transaction

The system handles burn address generation and recipient notification.

## Privacy Checklist

Before making a private transfer, review this checklist:

- [ ] **Amount**: Is the amount generic enough? Consider rounding.
- [ ] **Timing**: Are you withdrawing immediately after receiving? Consider waiting.
- [ ] **Address reuse**: Are you withdrawing to a fresh address? Consider using a new one.
- [ ] **Batching**: Can you combine multiple transfers? Reduces fingerprinting.
- [ ] **Chain**: Are you withdrawing on a different chain? Adds another layer of privacy.

## Advanced: CLI Private Transfer

For maximum control over privacy parameters:

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
  --amount 95 \  # Partial withdrawal
  --batch        # Combine with other pending transfers
```

See [CLI Guide](cli-guide.md) for full command reference.
