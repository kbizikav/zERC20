# Trusted Setup

Tools for running the trusted setup ceremonies that produce Nova and Groth16
artifacts for zERC20. This folder contains the coordinator service and the CLI
used by participants.

## Components
- `trusted-setup/coordinator`: Actix service that manages ceremonies, leases
  steps, and stores transcripts in S3 with SQLite state.
- `trusted-setup/cli`: Client for downloading the ptau file, contributing to a
  ceremony, and finalizing artifacts.

## Quick start
Coordinator:
1. Copy `trusted-setup/coordinator/.env.example` to `trusted-setup/coordinator/.env`
   and set `TRUSTED_SETUP_S3_BUCKET` plus AWS credentials as needed.
2. Run from the coordinator directory so `.env` is picked up:
   ```bash
   cd trusted-setup/coordinator
   cargo run -p trusted-setup-coordinator
   ```

CLI:
1. Copy `trusted-setup/cli/.env.example` to `trusted-setup/cli/.env` and set
   `TRUSTED_SETUP_COORDINATOR_URL`, `TRUSTED_SETUP_CEREMONY_ID`, and
   `TRUSTED_SETUP_CIRCUIT`.
2. Run from the CLI directory:
   ```bash
   cd trusted-setup/cli
   cargo run -p trusted-setup-cli -- ptau download
   cargo run -p trusted-setup-cli -- contribute
   cargo run -p trusted-setup-cli -- finalize
   ```

## Circuit values
- `contribute`: `withdraw_local`, `withdraw_global`, `decider_root`,
  `decider_withdraw_local`, `decider_withdraw_global`
- `finalize`: `root`, `withdraw_local`, `withdraw_global`

## Notes
- The default ptau path is `~/.cache/zerc20/ptau/ppot_0080_24.ptau`.
- The coordinator uses S3 presigned URLs; ensure AWS credentials and region are
  available in the environment.
