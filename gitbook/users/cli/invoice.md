# Invoice Flow

Invoices allow you to receive private payments where even the sender doesn't know your withdrawal address.

## How It Works

1. You generate a **burn address** (invoice) using the CLI
2. You share this burn address with the payer
3. The payer sends zERC20 to the burn address using any wallet
4. You withdraw the funds to any address you choose

The payer only sees the burn address—not your final withdrawal destination.

## Creating an Invoice

```bash
zerc20 invoice create --token zUSDC --amount 100
```

Output:
```
Invoice created!

Burn Address: 0x1234567890abcdef1234567890abcdef12345678
Token: zUSDC
Expected Amount: 100

Share the burn address with the payer.
Invoice saved to: ~/.zerc20/invoices/invoice-2024-01-15.json
```

### Options

```bash
# Specify the chain
zerc20 invoice create --token zUSDC --amount 100 --chain arbitrum

# Add a memo/note (stored locally)
zerc20 invoice create --token zUSDC --amount 100 --memo "Payment for services"
```

## Sharing the Invoice

Share the burn address with the payer through any channel:

- Messaging app
- Email
- QR code
- Payment link

The burn address looks like a normal Ethereum address:
```
0x1234567890abcdef1234567890abcdef12345678
```

The payer can send from any wallet on any supported chain.

## Listing Invoices

```bash
zerc20 invoice list
```

Output:
```
Your invoices:

ID      Token   Amount  Status      Created
─────────────────────────────────────────────
inv-01  zUSDC   100     Pending     2024-01-15
inv-02  zETH    0.5     Paid        2024-01-14
inv-03  zUSDC   50      Withdrawn   2024-01-10
```

## Checking Invoice Status

```bash
zerc20 invoice status inv-01
```

Output:
```
Invoice: inv-01
Burn Address: 0x1234...5678
Token: zUSDC
Expected: 100
Received: 100 (confirmed)
Status: Ready to withdraw
```

## Withdrawing Invoice Funds

Once payment is received:

```bash
# Withdraw to your address
zerc20 withdraw --invoice inv-01 --to 0xYourAddress

# Or withdraw to a different chain
zerc20 withdraw --invoice inv-01 --to 0xYourAddress --chain optimism
```

## Best Practices

1. **One invoice per payment**: Create a new invoice for each expected payment
2. **Don't reuse burn addresses**: Each burn address should only be used once
3. **Save invoice data**: The CLI stores invoice data locally; back it up if needed
4. **Verify amounts**: Confirm the received amount before withdrawing

## Troubleshooting

### Invoice not showing as paid

```bash
# Force rescan
zerc20 scan --force

# Check specific invoice
zerc20 invoice status inv-01 --verbose
```

### Payment sent to wrong amount

Partial payments work fine. If the payer sent less than expected, you can still withdraw what was received.
