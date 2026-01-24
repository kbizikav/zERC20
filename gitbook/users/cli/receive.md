# Receiving Funds

This guide covers how to scan for and withdraw incoming private transfers using the CLI.

## Scanning for Transfers

The CLI scans for incoming transfers to addresses you control:

```bash
zerc20 scan
```

Output:
```
Scanning for incoming transfers...

Found 3 transfers:

Index  Token   Amount      From (Burn Address)     Status
──────────────────────────────────────────────────────────────
1      zUSDC   100.00      0x1234...abcd           Ready
2      zUSDC   50.00       0x5678...efgh           Ready
3      zETH    0.25        0x9abc...ijkl           Pending (15 min)

Total available: 150 zUSDC, 0.25 zETH
```

### Scan Options

```bash
# Force a full rescan
zerc20 scan --force

# Scan specific token only
zerc20 scan --token zUSDC

# Show verbose output
zerc20 scan --verbose
```

## Withdrawing Funds

### Basic Withdrawal

```bash
zerc20 withdraw --to 0xYourAddress
```

This withdraws all available funds to the specified address.

### Withdraw Specific Amount

```bash
zerc20 withdraw --to 0xYourAddress --amount 100 --token zUSDC
```

### Withdraw to Different Chain

```bash
zerc20 withdraw --to 0xYourAddress --chain optimism
```

### Batch Withdrawal

Combine multiple transfers into one withdrawal (recommended for privacy):

```bash
zerc20 withdraw --to 0xYourAddress --batch
```

Only the total amount is visible on-chain, not individual transfer amounts.

### Partial Withdrawal

Withdraw less than received to obscure amount correlation:

```bash
# Received 123.456789 zUSDC
zerc20 withdraw --to 0xYourAddress --amount 120 --token zUSDC
# Remaining 3.456789 stays available for later
```

## Withdrawal Status

Check the status of pending withdrawals:

```bash
zerc20 status
```

Output:
```
Pending withdrawals:

ID          Amount      Destination         Status
────────────────────────────────────────────────────
wd-001      100 zUSDC   0xabcd...1234       Processing (ETA: 30 min)
wd-002      0.5 zETH    0xefgh...5678       Confirmed
```

## Best Practices

### For Privacy

1. **Batch when possible**: Combine multiple incoming transfers
2. **Use round amounts**: Withdraw 100 instead of 99.87
3. **Vary timing**: Don't withdraw immediately after receiving
4. **Fresh addresses**: Withdraw to addresses not linked to your identity
5. **Cross-chain**: Withdraw on a different chain than the sender used

### For Security

1. **Verify addresses**: Double-check withdrawal addresses
2. **Start small**: Test with small amounts first
3. **Backup data**: The CLI stores transfer data locally

## Troubleshooting

### Transfer not appearing

```bash
# Force a full rescan
zerc20 scan --force

# Check with verbose output
zerc20 scan --verbose
```

Private transfers take 30 minutes to 1 hour to process. On testnets, this may take longer.

### Withdrawal stuck

```bash
# Check status
zerc20 status --verbose

# Check specific withdrawal
zerc20 status wd-001
```

Cross-chain withdrawals require root synchronization across chains.
