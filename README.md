# zERC20

Before starting any node, make sure the Nova artifacts and Solidity verifiers exist. Run these commands from the repo root:

1. Download artifacts using `circuit-setup` CLI:

   ```bash
   cargo install --path circuit-setup
   zerc20-circuit-setup download --version <VERSION> --base-url <ARTIFACTS_URL>
   ```

   This downloads all circuit artifacts (`*_nova_pp.bin`, `*_nova_vp.bin`, `*_decider_pp.bin`, `*_decider_vp.bin`, `*_groth16_pk.bin`, `*_groth16_vk.bin`) into `nova_artifacts/` and verifies SHA256 hashes against the manifest.

   See `circuit-setup/README.md` for more details on available commands (generate, upload, test, generate-verifier).

2. Generate and copy the Solidity verifiers into the contracts package:
   ```bash
   zerc20-circuit-setup generate-verifier
   ./scripts/copy_nova_verifiers.sh
   ```
   The first command generates verifier contracts from the downloaded artifacts. The script copies every `*_verifier.sol` and `*.sol` into `contracts/src/verifiers/`, creating the folder if needed.

## Local setup guide

Follow these steps to bring up the indexer, crosschain job, decider-prover, and relay node, then exercise the CLI end-to-end:

1. Prepare token metadata
   Use a token configuration file from `config/deployed/` for your target environment (e.g., `config/deployed/mainnet/` or `config/deployed/testnet/`). Alternatively, copy `config/tokens.example.json` and fill it with your own deployed contracts.

2. Configure root environment
   Copy `.env.example` at the repo root to `.env`, then set the following environment variables:
   - `ALCHEMY_KEY` - Alchemy API key for RPC access
   - `INFURA_KEY` - Infura API key for RPC fallback access
   - `ROOT_SUBMITTER_PRIVATE_KEY` - Key for submitting roots on-chain
   - `RELAY_PRIVATE_KEY` - Key for relay node operations (`/relay/teleport`, `/relay/swap`)
   - `FEE_MANAGER_PRIVATE_KEY` - Key for fee manager operations
   - `TOKENS_FILE_PATH` - Path to token configuration file

   Optional relay-node envs:
   - `SWAP_ENABLED` - Enable `/relay/swap*` endpoints (`false` by default)
   - `SWAP_FEE_BPS` - Swap fee in basis points (`50` by default)
   - `MAX_SWAP_NATIVE_WEI` - Per-swap native output cap (`0.01 ETH` by default)

   These keys must control accounts with enough ETH on the EVM chains listed in your tokens configuration.

   If `SWAP_ENABLED=true`, every token entry must include `swap_helper_address` in `tokens.json`.

   ```bash
   # Example: Point to mainnet token configuration
   TOKENS_FILE_PATH=./config/deployed/mainnet/tokens.json
   ```

3. Start indexer and crosschain job  
   From the repo root, start the dockerized services:

   ```bash
   docker compose up -d
   ```

   Health check the indexer at `curl http://localhost:8080/healthz`.

4. Start the decider-prover PostgreSQL
   The decider-prover requires its own PostgreSQL instance. Start it with:

   ```bash
   docker compose -f docker-compose.decider.yml up -d
   ```

   This runs PostgreSQL on port 5433 (separate from the indexer's database on 5432).

5. Run the decider-prover
   In `decider-prover/`, copy `.env.example` to `.env`, then start the server:

   ```bash
   cargo run -r
   ```

   Health check at `curl http://localhost:8081/healthz`.

   > **Note:** The decider-prover must run directly on the host, not in Docker. It crashes during proof generation when containerized.

6. Run the relay node
   In `relay/`, start the HTTP server:

   ```bash
   cargo run -r
   ```

   Check it with:

   ```bash
   curl http://localhost:3000/relay/info
   ```

7. Exercise the CLI
   Use the CLI to send transfers, redeem via the relay node, and swap zERC20 into native gas tokens; see `cli/README.md` for commands and options.

## Contract-only testnet verification

If you only need to deploy contracts and do a quick on-chain smoke test (Hub + Verifier/zERC20 + Liquidity/Adaptor),
follow `contracts/README.md`.
