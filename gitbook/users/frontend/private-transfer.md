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
2. Click the "Private Send" tab
3. Select the recipient chain from the dropdown
4. Enter the recipient's Ethereum address
5. Enter the amount of zERC20 to send
6. Click "Send Privately"

![Private Send Form](../../assets/frontend_how_to/private-send-form.png)

The transfer will process in three steps:

![Transfer Progress](../../assets/frontend_how_to/transfer-progress.png)

Once completed, you'll see a success message:

![Transfer Success](../../assets/frontend_how_to/transfer-success.png)

The system generates an encrypted announcement, stores it, and transfers your tokens.

### To a Burn Address

If someone provides you with a burn address:

1. Open MetaMask (or any wallet)
2. Send zERC20 to the provided burn address
3. Done — the recipient handles the withdrawal

You can send from any supported chain.

## Creating a Burn Address for Others

To let someone pay you (or pay on behalf of a recipient):

1. Go to the Send page
2. Enter the recipient's withdrawal address and amount
3. Check the **"Pay with mobile"** option
4. Click "Pay with mobile"

![Pay with Mobile Form](../../assets/frontend_how_to/pay-with-mobile-form.png)

5. A QR code will be generated containing the burn address
6. Share the QR code with anyone who needs to pay

![Pay with Mobile QR](../../assets/frontend_how_to/pay-with-mobile-qr.png)

## Receiving and Withdrawing

See [Scan Receives](scan-receives.md) for detailed instructions on receiving and withdrawing private transfers.
