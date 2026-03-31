# Scan Receives (Frontend)

This guide explains how to scan for and receive private transfers using the zERC20 web application.

## Accessing Private Receive

1. Visit the [Frontend](https://app.zerc20.io/)
2. Connect your wallet
3. Click the "Private Receive" tab

<figure><img src="../../assets/frontend_how_to/private-receive-tab.png" alt="Private Receive Tab" width="560"><figcaption>Private Receive Tab</figcaption></figure>

## Scanning for Incoming Transfers

To check for incoming private transfers:

1. Click the "Scan for Receive" button
2. The system will request your view key and scan for announcements

<figure><img src="../../assets/frontend_how_to/scanning-progress.png" alt="Scanning Progress" width="480"><figcaption>Scanning Progress</figcaption></figure>

The scanning process:
- Requests your view key from your wallet (this key allows decryption of announcements addressed to you)
- Searches for encrypted announcements across all supported chains
- Decrypts announcements addressed to your wallet
- Does not reveal your private key or spending capability

## Viewing Received Transfers

After scanning, you'll see a list of incoming transfers grouped by timestamp:

<figure><img src="../../assets/frontend_how_to/announcement-list.png" alt="Announcement List" width="560"><figcaption>Announcement List</figcaption></figure>

Click the **+** button on an announcement to expand its details.

## Announcement Details

Each announcement goes through the following status flow:

| Status | Meaning |
|--------|---------|
| **PENDING** | Transfer detected but not yet confirmed for withdrawal |
| **READY** | Transfer confirmed and available for withdrawal |
| **REDEEMED** | Funds have already been withdrawn |

Click on an announcement to view its details:

<figure><img src="../../assets/frontend_how_to/announcement-detail.png" alt="Announcement Detail" width="480"><figcaption>Announcement Detail (PENDING)</figcaption></figure>

The detail view shows:
- **Status**: Current status of the transfer (PENDING / READY / REDEEMED)
- **Destination chain**: The chain where funds can be withdrawn
- **Amount and token**: The amount of zERC20 received
- **Source**: The originating chain and sender address
- **Burn address**: The burn address used for the transfer

## Withdrawing Funds

Once a transfer reaches **READY** status (shown as "ARRIVED"):

1. Expand the announcement
2. Wait for redeem resources to finish loading — the button shows **"Preparing redeem resources..."** while the zero-knowledge proof artifacts are being downloaded
3. Choose one of the available redeem paths:
   - **Direct redeem** — you submit the on-chain transaction yourself
   - **Use Relayer** — the relay node submits the redeem transaction on your behalf
4. If you use the relayer path, review the quoted relay fee and approve the confirmation dialog
5. Click the "REDEEM" button once it becomes active

<figure><img src="../../assets/frontend_how_to/redeem-detail.png" alt="Redeem Detail" width="480"><figcaption>Transfer ready for redemption</figcaption></figure>

6. Wait for the proof generation and transaction to complete (this creates a zero-knowledge proof that you are entitled to withdraw the funds)

7. Once completed, the status will change to **REDEEMED** and the button will show "Already Redeemed"

<figure><img src="../../assets/frontend_how_to/redeem-success.png" alt="Redeem Success" width="480"><figcaption>Transfer successfully redeemed</figcaption></figure>

The funds will be transferred to your connected wallet address.

### When to use the relayer path

Use **Use Relayer** when the destination chain wallet does not already have enough native gas to
submit the redeem transaction directly.

Notes:

- The relay fee is deducted from the redeemed zERC20 amount
- The frontend asks for confirmation before signing the relayer authorization
- The relay path still prepares the same proof resources; it only changes who submits the final
  on-chain transaction

## Troubleshooting

### No Announcements Found

If scanning shows no results:
- Ensure you're connected with the correct wallet
- Wait for the transfer to be confirmed on-chain (may take a few minutes)
- Check that the sender used the correct recipient address

### REDEEM Button Stays Disabled

If the REDEEM button remains disabled even though the transfer shows READY / ARRIVED:

- **"Preparing redeem resources..."** — The proof artifacts are still downloading. Wait for the download to complete.
- **"Failed to load redeem resources..."** — The artifact download failed. Use the reload button on the announcement detail to retry. This clears the stale cache and re-fetches both the aggregation state and proof artifacts.
- **Relayer fee estimate failed** — The relay node could not provide a fee quote. Retry, or switch off **Use Relayer** and redeem directly if you already have native gas.

> **Tip:** If a reload doesn't help, try refreshing the browser page. This resets all in-memory caches.

## Next Steps

- [Private Transfer](private-transfer.md) - Learn how to send private transfers
- [Getting Started](getting-started.md) - Overview of the frontend
- [FAQ](../faq.md) - Common questions and troubleshooting
