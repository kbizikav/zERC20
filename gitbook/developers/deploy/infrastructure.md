# Infrastructure Setup

After deploying contracts and creating `tokens.json`, set up the off-chain services: **Indexer**, **Decider Prover**, **Crosschain Job**, and **Fee Manager**.

## Circuit Artifacts

Download pre-built Nova circuit artifacts using the `zerc20-circuit-setup` tool:

```bash
cd circuit-setup
cp .env.example .env
# Edit .env: set NOVA_ARTIFACTS_DIR=../nova_artifacts
cargo run -- download
```

**Environment variables:**

| Variable | Description |
|----------|-------------|
| `ARTIFACTS_VERSION` | Version tag (e.g., `"1.1.0"`) |
| `NOVA_ARTIFACTS_DIR` | Output directory (default: `../nova_artifacts`) |
| `ARTIFACTS_BASE_URL` | S3 public URL for downloading artifacts |

The artifacts are required by both the **Indexer** (root prover) and the **Decider Prover**.

---

## Indexer

The indexer syncs on-chain transfer events, builds Poseidon Merkle trees, and generates Nova IVC root proofs.

### PostgreSQL

The indexer requires PostgreSQL 16+.

```bash
docker run -d --name zerc20-postgres \
  -e POSTGRES_USER=postgres \
  -e POSTGRES_PASSWORD=password \
  -e POSTGRES_DB=zerc20 \
  -p 5432:5432 postgres:16
```

### Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `DATABASE_URL` | Yes | PostgreSQL connection string |
| `TOKENS_FILE_PATH` | Yes | Path to tokens.json |
| `IS_SYNC` | No | Set to `"true"` to enable background sync jobs |
| `LISTEN_ADDR` | No | HTTP server address (default: `127.0.0.1:8080`) |
| `EVENT_INTERVAL_MS` | No | Event sync interval |
| `TREE_INTERVAL_MS` | No | Tree ingestion interval |
| `ROOT_INTERVAL_MS` | No | Root prover interval |
| `ROOT_SUBMITTER_PRIVATE_KEY` | Yes | Key for submitting root proofs on-chain |
| `DECIDER_PROVER_URL` | No | Decider service URL (if using batch proofs) |

### Running

```bash
cd indexer
cp .env.example .env
# Edit .env with your configuration
cargo run -- --listen-addr 0.0.0.0:8080
```

Or with Docker: see `docker-compose.yml`.

---

## Decider Prover

The decider prover converts Nova IVC proofs into Groth16 proofs for on-chain verification. It runs as a separate service with its own PostgreSQL database (to avoid conflicts with the indexer).

> **Important**: Run the decider prover on the host machine, not in Docker, due to memory-intensive proof generation.

### PostgreSQL for Decider

The decider uses a separate PostgreSQL instance (port 5433) to avoid conflicts with the indexer:

```bash
docker compose -f docker-compose.decider.yml up -d
```

### Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `DATABASE_URL` | Yes | PostgreSQL connection string (port 5433) |
| `ARTIFACTS_DIR` | Yes | Path to Nova parameters (e.g., `./nova_artifacts`) |
| `LISTEN_ADDR` | No | HTTP server address (default: `0.0.0.0:8081`) |
| `ENABLED_CIRCUITS` | No | Comma-separated: `root,withdraw_global` |
| `WORKER_COUNT` | No | Number of worker threads (default: 1) |
| `QUEUE_NAME` | No | Job queue name (default: `prover_queue`) |
| `JOB_TABLE` | No | Job table name (default: `prover_jobs`) |
| `JOB_TTL_SECONDS` | No | Job time-to-live (default: 86400) |
| `JSON_BODY_LIMIT_BYTES` | No | Max request body (default: 40MB) |

### Running

```bash
cd decider-prover
cp .env.example .env
# Edit .env
cargo run --release
```

### API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/jobs` | `POST` | Submit a proof job |
| `/jobs/{job_id}` | `GET` | Check job status |
| `/healthz` | `GET` | Health check |

---

## Crosschain Job

Relays transfer roots between chains and broadcasts aggregation updates via LayerZero.

### Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `RELAY_PRIVATE_KEY` | Yes | Key for relay transactions |
| `TOKENS_FILE_PATH` | Yes | Path to tokens.json |
| `RELAY_INTERVAL_SECS` | No | Relay interval (from tokens.json if not set) |
| `BROADCAST_INTERVAL_SECS` | No | Broadcast interval (from tokens.json if not set) |
| `LZSCAN_API_KEY` | No | LayerZero Scan API key for delivery confirmation |

### Running

```bash
cd crosschain-job
cp .env.example .env
cargo run
```

Use the `--once` flag for single execution.

---

## Fee Manager

Dynamically adjusts `targetLiquidity` across LiquidityManager contracts based on current balances.

### Prerequisites

The deployer wallet must hold the `FEE_MANAGER_ROLE` on each LiquidityManager.

### Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `FEE_MANAGER_PRIVATE_KEY` | Yes | Key with `FEE_MANAGER_ROLE` |
| `TOKENS_FILE_PATH` | Yes | Path to tokens.json |
| `INTERVAL_SECS` | No | Update interval in seconds |
| `K` | No | Incentive strength coefficient (basis points) |

### Running

```bash
cd fee-manager
cp .env.example .env
cargo run
```

---

## Running the Full Stack

Using Docker Compose (indexer + crosschain-job + postgres):

```bash
# Set required env vars
export ALCHEMY_KEY=your_key
export ROOT_SUBMITTER_PRIVATE_KEY=0x...
export RELAY_PRIVATE_KEY=0x...
export TOKENS_FILE_PATH=./config/tokens.json

docker compose up -d
```

For the decider prover (separate):

```bash
docker compose -f docker-compose.decider.yml up -d  # postgres only
cd decider-prover && cargo run --release              # run on host
```

---

## Monitoring

- All services support the `RUST_LOG` env var (e.g., `info`, `debug`).
- **Indexer**: `GET /healthz` and `GET /status` endpoints.
- **Decider**: `GET /healthz` endpoint.
- Check logs with `docker compose logs -f <service>`.

### Troubleshooting

| Issue | Cause | Solution |
|-------|-------|----------|
| Indexer not syncing events | `IS_SYNC` not set | Set `IS_SYNC=true` |
| Root prover fails | Missing circuit artifacts | Run `circuit-setup download` |
| Decider OOM | Insufficient RAM | Increase host memory, reduce `WORKER_COUNT` |
| Crosschain relay fails | Insufficient gas | Fund relay wallet on all chains |

---

**See also:** [Overview](overview.md) | [Contract Deployment](contracts.md) | [End-to-End Walkthrough](end-to-end.md)
