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

## Local setup guide

Follow these steps to bring up the indexer, crosschain job, and decider-prover, then exercise the CLI end-to-end:

1. Prepare token metadata  
   Copy `config/tokens.example.json` to `config/tokens.json` and fill it with the chains/tokens you want to use. You can point to contracts you deploy yourself or to an already-deployed environment.

2. Configure root environment  
   Copy `.env.example` at the repo root to `.env`, then set `ROOT_SUBMITTER_PRIVATE_KEY` and `RELAY_PRIVATE_KEY`. These keys must control accounts with enough testnet ETH on the EVM chains listed in `config/tokens.json`. Next, compress the token config and set `TOKENS_COMPRESSED`:

   ```bash
   ./scripts/encode_tokens.sh                 # reads config/tokens.json by default
   # paste the printed value into TOKENS_COMPRESSED in your .env
   ```

3. Start indexer and crosschain job  
   From the repo root, start the dockerized services:

   ```bash
   docker compose up -d
   ```

   Health check the indexer at `curl http://localhost:8080/healthz`.

4. Run the decider-prover  
   In `decider-prover/`, copy `.env.example` to `.env`, then start the server:

   ```bash
   cargo run -r
   ```

   Health check at `curl http://localhost:8081/healthz`.

5. Exercise the CLI  
   Use the CLI to send transfers and receive funds; see `cli/README.md` for commands and options.
