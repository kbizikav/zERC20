# zERC20

Before starting any node, make sure the Nova artifacts and Solidity verifiers exist. Run these commands from the repo root:

1. Generate artifacts with the release build:

   ```bash
   cargo run --release --bin generate_circuit_artifacts
   ```

   This fills `nova_artifacts/` with the Nova folding artifacts (`*_nova_pp.bin`, `*_nova_vp.bin`, `*_decider_pp.bin`, `*_decider_vp.bin`, `*_verifier.sol`) and the Groth16 withdraw artifacts (`*_groth16_pk.bin`, `*_groth16_vk.bin`, `*_groth16_verifier.sol`).

2. Copy the Solidity verifiers into the contracts package:
   ```bash
   ./scripts/copy_nova_verifiers.sh
   ```
   The script copies every `*_verifier.sol` into `contracts/src/verifiers/`, creating the folder if needed.

## Docker Orchestration

Container definitions for Postgres plus the `decider-prover`, `tree-indexer`, and `crosschain-job` services live under `docker/` with a root-level `docker-compose.yml`. Build and start everything with:

```bash
docker compose up --build
```
