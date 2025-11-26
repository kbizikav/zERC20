# zERC20 CLI

 Rust CLI for sending zERC20 transfers and redeeming stealth burns. Load environment variables from
 `.env` (see `.env.example`); set `TOKENS_FILE_PATH` to `../config/tokens.json` (or another tokens
 file), and provide the Internet Computer endpoints: `IC_REPLICA_URL`, `KEY_MANAGER_CANISTER_ID`,
 and `STORAGE_CANISTER_ID` must be set (env vars or CLI flags). Run commands from the `cli/`
 directory:

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
5. Generate proofs and redeem:
   ```bash
   cargo run -r -- invoice receive \
     --chain-id <CHAIN_ID> \
     --invoice-id <INVOICE_ID_FROM_LS>
   ```
