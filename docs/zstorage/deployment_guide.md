# zstorage Deployment Guide

This document explains how to deploy the `zstorage` Rust canisters (`key_manager` and `storage`) to the Internet Computer (IC). It covers both the local replica (`dfx` `local` network) and the production `ic` network.

## 1. Prerequisites

- Complete the environment setup in `docs/environment_setup.md`.
- Confirm `dfx --version` matches the project requirement (for example `0.29.2`).
- Ensure the Rust toolchain includes the `wasm32-unknown-unknown` target.
- Have sufficient cycles available for production deployments.

## 2. Deployment Basics

- `key_manager` requires an initialization argument `key_id_name` (the VetKD key name).
- `storage` accepts an optional `capacity_hint` initialization argument. Omit it by passing `null`.
- Running `dfx deploy` refreshes `.env` with `CANISTER_ID_*` values—load this file from clients that call the canisters.

## 3. Managing `dfx` Identities

1. **Create an identity**

   ```bash
   dfx identity new production
   ```

   - By default the secret key is stored encrypted in `~/.config/dfx/identity/<name>/`.
   - Use `--disable-encryption` only for ephemeral testing identities.

2. **Back up the identity**

   ```bash
   dfx identity export production > production.pem
   ```

   - Store exports in an offline vault. Protect the file permissions (`chmod 600`).

3. **Import an existing identity (for new machines)**

   ```bash
   dfx identity import production production.pem
   ```

4. **Inspect principals**

   ```bash
   dfx identity use production
   dfx identity get-principal
   ```

5. **Provision or attach a wallet**

   ```bash
   dfx identity get-wallet --network ic
   ```

   - If a wallet is already registered, the command prints its canister ID and stores it in `~/.config/dfx/identity/<name>/wallets.json`.
   - When it reports `No wallet configured`, follow the guidance below.

6. **Create a wallet canister when none exists**

   1. Display the controller principal:
      ```bash
      dfx identity get-principal --network ic
      ```
   2. Transfer ICP to that principal (for example through the NNS dapp).
   3. Use the ledger to create a wallet canister funded with cycles:
      ```bash
      dfx ledger --network ic create-canister <controller_principal> --amount 0.5
      ```
      - Replace `<controller_principal>` with the value from step 1.
      - Adjust `--amount` to the ICP you want to convert (the ledger burns it into cycles during creation).
      - The command prints the new canister ID (format `rrkah-fqaaa-...`).
   4. Claim the canister as your wallet:
      ```bash
      dfx identity deploy-wallet --network ic <CANISTER_ID>
      ```
      - After deployment, rerun `dfx identity get-wallet --network ic` to confirm linkage.
   5. Optional: run the guided helper, which performs the same setup interactively:
      ```bash
      dfx quickstart --identity production --network ic
      ```

7. **Check cycles balance**

   ```bash
   dfx wallet --network ic balance
   ```

   - If the balance call fails, the wallet was not linked; return to step 6.

8. **List and clean up identities**
   ```bash
   dfx identity list
   dfx identity remove old-test-identity
   ```
   - Deleting an identity only removes local credentials; it does not revoke deployed canisters. Rotate principals manually if needed.

## 4. Deploying to the Local Replica (`dfx local`)

1. **Start the replica**

   ```bash
   dfx start --background --clean --host 127.0.0.1:4943
   ```

   - Drop `--clean` if you want to preserve existing state.

2. **Optional pre-build**  
   `dfx deploy` triggers a build automatically, but you can run it manually:

   ```bash
   cargo build --target wasm32-unknown-unknown --release
   ```

3. **Deploy `key_manager`**

   ```bash
   dfx deploy key_manager \
     --network local \
     --argument '(record { key_id_name = "local-test-key" })'
   ```

   - Use any string for `key_id_name` during local testing.

4. **Deploy `storage`**

   ```bash
   dfx deploy storage \
     --network local \
     --argument '(opt record { capacity_hint = opt 1_000 })'
   ```

   - Pass `--argument '(null)'` to leave `capacity_hint` unset.

5. **Verify the deployment**

   ```bash
   dfx canister status key_manager --network local
   dfx canister status storage --network local
   dfx canister call storage list_announcements '(null, null)' --network local
   ```

6. **Stop the replica**
   ```bash
   dfx stop
   ```

## 5. Deploying to Production (`dfx ic`)

> Before deploying to `ic`, make sure the active `dfx identity` has access to a wallet with enough cycles.

1. **Confirm the identity**

   ```bash
   dfx identity list
   dfx identity use production
   dfx identity get-principal
   ```

2. **Check wallet balance (if needed)**

   ```bash
   dfx wallet --network ic balance
   ```

3. **Deploy `key_manager` to production**

   ```bash
   export KEY_ID_NAME="ic-prod-key"  # example
   dfx deploy key_manager \
     --network ic \
     --argument "(record { key_id_name = \"${KEY_ID_NAME}\" })"
   ```

4. **Deploy `storage` to production**

   ```bash
   dfx deploy storage \
     --network ic \
     --argument '(opt record { capacity_hint = null })'
   ```

   - Use `dfx deploy storage --mode upgrade ...` to keep existing state during upgrades.

5. **Confirm the results**
   ```bash
   dfx canister status key_manager --network ic
   dfx canister status storage --network ic
   ```

## 6. Canister Balance Management

Keeping canisters funded is as important as deploying them. Follow these routines whenever you move to a new environment or schedule maintenance.

1. **Check the active identity and wallet**

   ```bash
   dfx identity whoami
   dfx identity get-wallet --network ic
   ```

   - Switch identities with `dfx identity use <name>` if necessary.
   - Omit `--network ic` on the local replica; the wallet is only required on the production `ic`.

2. **Inspect canister cycles**

   ```bash
   dfx canister status <CANISTER_NAME_OR_ID> --network ic
   ```

   - The `Balance: … Cycles` line shows the available cycles for the canister.
   - `dfx canister status` also reports module info, controller principals, and memory usage—handy for quick health checks.

3. **Top up from the dfx wallet**

   ```bash
   dfx wallet balance --network ic
   dfx wallet send <AMOUNT> <CANISTER_ID> --network ic
   ```

   - `<AMOUNT>` accepts suffixes such as `1T` for one trillion cycles.
   - Leave enough cycles in the wallet to pay transaction fees (several billion cycles); otherwise the transfer can fail.
   - Re-run `dfx wallet balance --network ic` and `dfx canister status <CANISTER_ID> --network ic` to confirm the new balances.

4. **Plan for the cycles ledger transition**

   - DFX 0.19.0 and later encourage managing cycles through the cycles ledger (`dfx cycles top-up`) instead of the legacy wallet, which is slated for removal.
   - For new production setups consider migrating to the ledger flow while keeping the wallet commands above for legacy baselines.

## 7. Upgrades and Redeployments

- Use `--mode upgrade` to retain state, or `--mode reinstall` to reset the canister.
  ```bash
  dfx deploy storage --mode upgrade --network local --argument '(null)'
  dfx deploy key_manager --mode reinstall --network local \
    --argument '(record { key_id_name = "local-test-key" })'
  ```
- After any redeploy, check `.env` for updated `CANISTER_ID_*` values and sync client configuration.

## 8. Troubleshooting

- **VetKD errors**: Ensure the `key_id_name` exists in the VetKD registry. Production deployments must use keys provisioned by DFINITY.
- **`nonce reuse detected`**: The `request_encrypted_view_key` method rejects repeated nonces per address. Keep nonces strictly increasing on the client.
- **Insufficient `capacity_hint`**: When importing large batches into `storage`, choose a larger hint to avoid repeated reallocations. Reinstall with a new hint if needed.

## 9. References

- [Internet Computer dfx CLI reference](https://internetcomputer.org/docs/current/references/cli-reference/dfx-parent)
- [Candid argument reference](https://internetcomputer.org/docs/current/references/candid-ref)
- [Cycles wallet management](https://internetcomputer.org/docs/building-apps/canister-management/cycles-wallet)
