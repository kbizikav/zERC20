# Deployment Guide

This guide covers deploying zERC20 infrastructure.

## Components

A full zERC20 deployment includes:

1. **Smart Contracts**: zERC20, Verifier, Hub, LiquidityManager
2. **Indexer**: Event sync + Merkle tree service
3. **Decider Prover**: Nova proof finalization
4. **Cross-chain Job**: Root relay automation
5. **Fee Manager**: Dynamic liquidity target adjustment
6. **ICP Canisters**: Stealth messaging (optional for self-hosted)

## Quick Start (Docker)

The fastest way to run the off-chain infrastructure:

```bash
git clone https://github.com/kbizikav/zERC20.git
cd zERC20/docker
cp .env.example .env
# Edit .env with your configuration
docker-compose up -d
```

This starts:
- PostgreSQL database
- Indexer service
- Cross-chain job
- Fee manager

## Deployment Guides

- [Contract Deployment](contracts.md) — Deploy smart contracts
- [Indexer Setup](indexer.md) — Run the indexer service

## Configuration

### Environment Variables

```bash
# Database
DATABASE_URL=postgres://user:pass@localhost:5432/zerc20

# RPC Endpoints
RPC_URL_ETHEREUM=https://eth-mainnet.g.alchemy.com/v2/...
RPC_URL_ARBITRUM=https://arb-mainnet.g.alchemy.com/v2/...

# Token Configuration
TOKENS_FILE_PATH=./config/tokens.json

# ICP (optional)
IC_REPLICA_URL=https://ic0.app
KEY_MANAGER_CANISTER_ID=...
STORAGE_CANISTER_ID=...

# Prover
NOVA_ARTIFACTS_DIR=./nova_artifacts

# Fee Manager
FEE_MANAGER_PRIVATE_KEY=0x...  # Key with FEE_MANAGER_ROLE
FEE_MANAGER_INTERVAL_SECS=3600
FEE_MANAGER_K_BPS=1000
```

### Token Configuration

`config/tokens.json` defines token metadata:

```json
{
  "tokens": [
    {
      "symbol": "zUSDC",
      "chains": [
        {
          "chainId": 1,
          "rpcUrl": "https://...",
          "zERC20": "0x...",
          "verifier": "0x..."
        }
      ],
      "hub": {
        "chainId": 42161,
        "address": "0x..."
      }
    }
  ]
}
```

## Infrastructure Requirements

### Indexer

- **CPU**: 2+ cores
- **Memory**: 4+ GB
- **Storage**: 50+ GB (grows with transfer volume)
- **Database**: PostgreSQL 14+

### Decider Prover

- **CPU**: 4+ cores (proof generation is CPU-intensive)
- **Memory**: 16+ GB
- **Storage**: 10+ GB (for Nova artifacts)

### Cross-chain Job

- **CPU**: 1 core
- **Memory**: 1 GB
- **Network**: Reliable connectivity to all chains

### Fee Manager

- **CPU**: 1 core
- **Memory**: 512 MB
- **Network**: Reliable connectivity to all chains with LiquidityManager

## Monitoring

### Health Checks

```bash
# Indexer
curl http://localhost:8080/health

# Decider prover
curl http://localhost:8081/health
```

### Metrics

Services expose Prometheus metrics:

- `indexer_synced_block`: Latest synced block per chain
- `indexer_tree_size`: Merkle tree leaf count
- `prover_jobs_completed`: Proof generation count

## Upgrades

### Contract Upgrades

Contracts use UUPS proxy pattern. See [Contract Deployment](contracts.md) for upgrade procedures.

### Service Upgrades

```bash
cd docker
docker-compose pull
docker-compose up -d
```

## Troubleshooting

### Indexer not syncing

1. Check RPC endpoint connectivity
2. Verify `TOKENS_FILE_PATH` configuration
3. Check database connection

### Proofs failing

1. Verify `NOVA_ARTIFACTS_DIR` contains valid artifacts
2. Check decider prover logs for errors
3. Ensure sufficient memory for proof generation

### Cross-chain delays

1. Check LayerZero explorer for message status
2. Verify gas funding on all chains
3. Check cross-chain job logs
