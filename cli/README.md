# zERC20 CLI

Rust CLI for sending zERC20 transfers, redeeming stealth burns, submitting gasless relay teleports,
and swapping zERC20 into native gas tokens. Load environment variables from `.env` (see
`.env.example`); set `TOKENS_FILE_PATH` to `../config/tokens.json` (or another tokens file), and
provide the Internet Computer endpoints: `IC_REPLICA_URL`, `KEY_MANAGER_CANISTER_ID`, and
`STORAGE_CANISTER_ID` must be set (env vars or CLI flags). Run commands from the `cli/` directory:

```bash
cargo run -r -- <command> ...
```

## Invoice flow (single)

Issue an invoice, fund it, check eligibility, and redeem with proofs (ensure `NOVA_ARTIFACTS_DIR`
is set in your environment before redemption).

1. Issue (single mode by default) and note the burn address + invoice ID:
   ```bash
   cargo run -r -- invoice issue --chain-id <CHAIN_ID>
   ```
2. List invoices for your address to confirm the ID:
   ```bash
   cargo run -r -- invoice ls --chain-id <CHAIN_ID>
   ```
3. Send funds to the printed burn address:
   ```bash
   cargo run -r -- transfer \
     --chain-id <CHAIN_ID> \
     --to <BURN_ADDRESS_FROM_ISSUE> \
     --amount 1000000000000000000
   ```
4. Check eligibility of transfers for the invoice:
   ```bash
   cargo run -r -- invoice status \
     --chain-id <CHAIN_ID> \
     --invoice-id <INVOICE_ID_FROM_LS>
   ```
   Use `--local` to inspect only the local proved transfer root on that chain instead of the hub global root:
   ```bash
   cargo run -r -- invoice status \
     --chain-id <CHAIN_ID> \
     --invoice-id <INVOICE_ID_FROM_LS> \
     --local
   ```
5. Generate proofs and redeem:
   ```bash
   cargo run -r -- invoice receive \
     --chain-id <CHAIN_ID> \
     --invoice-id <INVOICE_ID_FROM_LS>
   ```
   Local-root redemption uses the same flag:
   ```bash
   cargo run -r -- invoice receive \
     --chain-id <CHAIN_ID> \
     --invoice-id <INVOICE_ID_FROM_LS> \
     --local
   ```

## Gasless redemption via relay node

If you want the relay node to submit the redeem transaction, pass `--relay` and point the CLI at
the relay HTTP server:

```bash
cargo run -r -- invoice receive \
  --chain-id <CHAIN_ID> \
  --invoice-id <INVOICE_ID> \
  --relay \
  --relay-url http://127.0.0.1:3000
```

Optional relay flags:
- `--max-relay-fee <AMOUNT>`: abort if the quoted relayer fee exceeds this zERC20 amount
- `--yes`: skip the confirmation prompt
- `--local`: redeem from the latest proved local root instead of the hub global root

## Swap zERC20 into native gas token

The `swap` command asks the relay node for a quote, signs an ERC-2612 permit for the chain-local
`SwapHelper`, and submits a token-to-native swap:

```bash
cargo run -r -- swap \
  --chain-id <CHAIN_ID> \
  --amount <TOKEN_AMOUNT> \
  --relay-url http://127.0.0.1:3000
```

Useful options:
- `--slippage-bps <BPS>`: set the minimum accepted native output (`0..=9999`, default `100`)
- `--recipient <ADDRESS>`: receive native tokens at a different address
- `--yes`: skip the confirmation prompt

Notes:
- The CLI prints `Swap submitted.` after the relay accepts the request and returns a transaction
  hash. This is submission, not final on-chain confirmation.
- If the relay quote reports `priceFallback`, the CLI prints a warning because fallback or stale
  oracle prices may be less favorable.
