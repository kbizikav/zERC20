# Trusted Setup

User-facing guide for running the trusted setup ceremonies that produce Nova and
Groth16 artifacts for zERC20. There are two roles:

- Coordinator operator: runs the service and initializes ceremonies.
- Participant: uses the CLI to contribute and finalize artifacts.

## Components
- `trusted-setup/coordinator`: Actix service that manages ceremonies, leases
  steps, and stores transcripts in S3 with SQLite state.
- `trusted-setup/cli`: Client for downloading the ptau file, contributing to a
  ceremony, and finalizing artifacts.

## Coordinator operator flow (start a new ceremony)
1. Configure environment.
   - Copy `trusted-setup/coordinator/.env.example` to
     `trusted-setup/coordinator/.env`.
   - Set at least `TRUSTED_SETUP_S3_BUCKET` and AWS credentials/region.
2. Start the coordinator (this only starts the service).
   ```bash
   cd trusted-setup/coordinator
   cargo run -p trusted-setup-coordinator
   ```
3. Initialize a ceremony (this actually creates the ceremony).
   ```bash
   curl -X POST \
     -H "Content-Type: application/json" \
     http://localhost:8080/api/ceremonies/<ceremony_id>/init \
     -d '{"circuit":"withdraw_local"}'
   ```
   Use `withdraw_local` or `withdraw_global` for Groth16 ceremonies.

After init, participants can begin contributing with the CLI.

## Participant flow (CLI)
1. Configure environment.
   - Copy `trusted-setup/cli/.env.example` to `trusted-setup/cli/.env`.
   - Set `TRUSTED_SETUP_COORDINATOR_URL`, `TRUSTED_SETUP_CEREMONY_ID`, and
     `TRUSTED_SETUP_CIRCUIT`.
2. Run the CLI from the CLI directory:
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
