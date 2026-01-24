# Scan Receives (Frontend)

This guide explains how to scan for and receive private transfers using the zERC20 web application.

## Accessing Private Receive

1. Visit the [Frontend](https://v2.testnet.app.zerc20.io/)
2. Connect your wallet
3. Click the "Private Receive" tab

![Private Receive Tab](../../assets/frontend_how_to/private-receive-tab.png)

## Scanning for Incoming Transfers

To check for incoming private transfers:

1. Click the "Scan for Receive" button
2. The system will request your view key and scan for announcements

![Scanning Progress](../../assets/frontend_how_to/scanning-progress.png)

The scanning process:
- Requests your view key from your wallet
- Searches for encrypted announcements across all supported chains
- Decrypts announcements addressed to your wallet

## Viewing Received Transfers

After scanning, you'll see a list of incoming transfers:

![Announcement List](../../assets/frontend_how_to/announcement-list.png)

Each announcement shows:
- **ID**: Unique identifier for the transfer
- **Transaction Hash**: The on-chain transaction reference
- **Timestamp**: When the transfer was made

## Announcement Details

Click on an announcement to view its details:

![Announcement Detail](../../assets/frontend_how_to/announcement-detail.png)

The detail view shows:
- **Announcement ID**: Unique identifier
- **Chain**: The chain where the transfer originated
- **Type**: Single or batch transfer
- **Eligible Value Total**: Amount already claimed
- **Pending Total Value**: Amount available to withdraw
- **Eligible Events**: Number of redeemable transfers

## Withdrawing Funds

Once you have eligible transfers:

1. Click on the announcement with pending value
2. Click the "REDEEM" button

![Redeem Detail](../../assets/frontend_how_to/redeem-detail.png)

3. Wait for the proof generation process

![Redeem Progress](../../assets/frontend_how_to/redeem-progress.png)

4. Wait for the transaction to complete

![Redeem Success](../../assets/frontend_how_to/redeem-success.png)

## Troubleshooting

### No Announcements Found

If scanning shows no results:
- Ensure you're connected with the correct wallet
- Wait for the transfer to be confirmed on-chain (may take a few minutes)

## Next Steps

- [Private Transfer](private-transfer.md) - Learn how to send private transfers
- [Getting Started](getting-started.md) - Overview of the frontend
- [FAQ](../faq.md) - Common questions and troubleshooting
