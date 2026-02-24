# zERC20 Watcher

A monitoring daemon that continuously checks the health of the zERC20 system and sends alerts to Discord. It covers three domains: **balance monitoring**, **indexer pipeline tracking**, and **crosschain root synchronization**.

## Architecture Overview

```
                          watcher (main loop)
                                |
              +-----------------+-----------------+
              |                 |                 |
         Balance           Indexer           Crosschain
         Monitor           Monitor            Monitor
              |                 |                 |
        RPC: balances    Indexer API +       Hub + Verifier
                         On-chain RPC       contract calls
              |                 |                 |
              +--------+--------+---------+-------+
                       |                  |
                 AlertManager        Stats Reporter
                       |                  |
                   Discord Webhook    Discord Webhook
```

The watcher runs on a configurable interval (default: 60s). Each cycle executes all three monitoring domains, collects alerts, applies cooldown-based deduplication (default: 1 hour), and posts filtered alerts to Discord. An optional stats reporter sends periodic system-wide status summaries.

## Quick Start

```bash
# 1. Copy and fill in environment variables
cp watcher/.env.example watcher/.env

# 2. Run in continuous mode
cargo run --bin zerc20-watcher -- --config watcher/watcher.yaml

# 3. Or run once and exit
cargo run --bin zerc20-watcher -- --config watcher/watcher.yaml --once
```

## Configuration

All configuration lives in `watcher.yaml`. Environment variables are supported via `${VAR_NAME}` syntax.

```yaml
discord_webhook_url: "${DISCORD_WEBHOOK_URL}"
interval_seconds: 60              # Main loop interval

# Balance monitoring
accounts:
  - name: mainnet_fee_manager
    address: "${MAINNET_FEE_MANAGER_ADDRESS}"
    required_balance: "0.01"      # ETH denomination
    chains: [ethereum, base, arbitrum]

chains:
  ethereum:
    rpc_url: "https://eth-mainnet.g.alchemy.com/v2/${ALCHEMY_KEY}"
    explorer: "https://etherscan.io/address/"

# Token-level monitoring (shared by indexer + crosschain)
tokens:
  - name: zUSDC
    indexer_url: "https://v1.mainnet.api.zerc20.io/indexer/zusdc"
    crosschain_config_path: "../config/deployed/mainnet/tokens.zusdc.mainnet.json"

# Indexer thresholds
indexer:
  stale_threshold_cycles: 5       # Alert after N consecutive stale cycles

# Crosschain thresholds
crosschain:
  root_delay_threshold_seconds: 2400  # 40 minutes

# Alert deduplication
alert:
  cooldown_seconds: 3600          # Suppress duplicate alerts for 1 hour

# Periodic stats report (0 or omit to disable)
stats_interval_seconds: 3600
```

### Environment Variables

| Variable | Description |
|---|---|
| `DISCORD_WEBHOOK_URL` | Discord webhook endpoint for alerts |
| `ALCHEMY_KEY` | Alchemy API key for RPC access |
| `MAINNET_FEE_MANAGER_ADDRESS` | Fee manager account address |
| `MAINNET_RELAYER_*_ADDRESS` | Relayer account addresses per token |

---

## Monitoring Domains

### 1. Balance Monitor

**Purpose:** Ensures critical service accounts (fee managers, relayers) maintain sufficient native token (ETH/BNB) balances to operate.

**What it checks:**
- Calls `provider.get_balance(address)` via RPC for each account on each configured chain.
- Compares the balance against the configured `required_balance` threshold.

**Alert condition:**
| Alert | Severity | Condition |
|---|---|---|
| Low balance | Warning | `balance < required_balance` |

**Why this matters:** Relayers and fee managers need ETH to submit transactions. If their balance drops too low, crosschain message delivery and proof submission will stall.

---

### 2. Indexer Monitor

**Purpose:** Detects when the indexer pipeline falls behind on-chain state. The indexer is responsible for tracking transfer events and feeding them to the prover. If the indexer stalls, new transfers won't be proved or included in aggregation roots.

**Data flow being monitored:**

```
On-chain transfers ──> Indexer (tree_synced) ──> Prover (latestProvedIndex)
     zERC20.index()        /status API           Verifier.latestProvedIndex()
```

#### Contracts and APIs queried

| Source | Call | Returns |
|---|---|---|
| Indexer API | `GET /healthz` | HTTP status (health check) |
| Indexer API | `GET /status` | `TokenStatusResponse[]` with `chain_id` and `tree_synced_index` per chain |
| zERC20 contract | `index()` | Current on-chain transfer tree index (number of deposits/transfers) |
| Verifier contract | `latestProvedIndex()` | Latest transfer tree index for which a ZK proof has been generated and verified |

#### Staleness detection

The monitor tracks values across consecutive check cycles and uses a **cycle counter** to detect stalls. A value is considered "stale" when it hasn't changed for `stale_threshold_cycles` consecutive checks (default: 5) AND the upstream value has advanced beyond it.

#### Alert conditions

| Alert | Severity | Condition | What it means |
|---|---|---|---|
| Indexer unhealthy | Critical | `/healthz` returns non-2xx HTTP status | Indexer service is returning errors |
| Indexer unreachable | Critical | `/healthz` network error | Indexer service is down or unreachable |
| Status fetch failed | Critical | `/status` request fails | Cannot retrieve indexer sync state |
| `tree_synced` stale | Warning | `tree_synced_index` unchanged for N cycles **AND** `zERC20.index() > tree_synced_index` | Indexer has stopped syncing new on-chain events while new transfers are happening |
| Proved index stale | Warning | `latestProvedIndex` unchanged for N cycles **AND** `tree_synced_index > latestProvedIndex` | ZK proofs are not being generated/submitted while the indexer has synced new data |

**Why this matters:**
- **tree_synced stale:** Transfers are happening on-chain but the indexer isn't picking them up. Users' transfers won't be included in proofs.
- **Proved index stale:** The indexer is up to date, but the prover pipeline has stalled. No new proofs are being submitted, which blocks aggregation root updates and crosschain settlement.

---

### 3. Crosschain Monitor

**Purpose:** Monitors the cross-chain root synchronization protocol. zERC20 uses a hub-and-spoke architecture where the Hub contract on Ethereum L1 broadcasts aggregation roots to Verifier contracts on each L2. Verifiers also relay transfer roots back to the Hub. This monitor checks both directions.

**Data flow being monitored:**

```
Direction 1: Hub → Verifiers (Aggregation Root Sync)
  Hub.aggSeq / AggregationRootUpdated event
       ──[LayerZero/bridge message]──>
  Verifier.latestAggSeq / globalTransferRoot(aggSeq)

Direction 2: Verifiers → Hub (Transfer Root Relay)
  Verifier.latestRelayedIndex / TransferRootRelayed event
       ──[LayerZero/bridge message]──>
  Hub.transferTreeIndex(position)
```

#### Contracts queried

| Contract | Method | Returns | Used for |
|---|---|---|---|
| Hub | `aggSeq()` | Current aggregation sequence number | Baseline: how many roots the hub has broadcast |
| Hub | `eidPosition(eid)` | Position of a verifier in the hub's tree | Mapping verifier to its slot in the hub |
| Hub | `transferTreeIndex(position)` | Transfer root index received at a given position | What the hub has received from a verifier |
| Hub | Event: `AggregationRootUpdated` | `root`, `block_timestamp` | Source of truth for broadcast root + timing |
| Verifier | `latestAggSeq()` | Latest aggregation sequence received | How far behind the verifier is |
| Verifier | `globalTransferRoot(aggSeq)` | Root stored at a given sequence | For root integrity comparison |
| Verifier | `latestRelayedIndex()` | Latest transfer root index relayed to hub | What the verifier has sent |
| Verifier | Event: `TransferRootRelayed` | `block_timestamp` | When a relay message was sent |

#### 3a. Root Sync Checks (Hub → Verifier)

These checks verify that aggregation roots broadcast by the Hub are correctly received by each Verifier.

**Comparison logic:**

1. Read `Hub.aggSeq()` — the total number of aggregation root updates.
2. For each Verifier, read `Verifier.latestAggSeq()`.
3. If both are at the same `aggSeq`: compare the roots.
   - The Hub root is obtained from the `AggregationRootUpdated` **event** (not `currentAggregationRoot()` which may have already advanced).
   - The Verifier root is read from `Verifier.globalTransferRoot(aggSeq)`.
4. If the Verifier is behind: measure the delay since the Hub broadcast the first missing root.

| Alert | Severity | Condition | What it means |
|---|---|---|---|
| Root not synced | Warning | `Verifier.latestAggSeq() == 0` | Verifier has never received any aggregation root. Either newly deployed or bridge delivery is completely broken. |
| **Root mismatch** | **Critical** | `Verifier.latestAggSeq() == Hub.aggSeq()` AND `Verifier.globalTransferRoot(seq) != Hub event root` AND `Verifier root != 0` | **Protocol-level inconsistency.** The Verifier and Hub disagree on the root at the same sequence number. This could indicate a bridge relay bug, a malicious message, or a contract state corruption. Requires immediate investigation. |
| Root sync delayed | Warning | `Verifier.latestAggSeq() < Hub.aggSeq()` AND the Hub broadcast event for the first missing sequence is older than `root_delay_threshold_seconds` (default: 40min) | The bridge message carrying the aggregation root to this L2 has not been delivered within the expected timeframe. The crosschain message relay may be stalled. |
| Root sync delayed (very old) | Warning | Same as above, but the Hub event is not found within the lookback window (50,000 blocks / ~7 days) | The verifier is extremely far behind — the root was broadcast so long ago it's no longer in the event scan window. |

**Why root mismatch is compared against the event root (not `currentAggregationRoot()`):**
The Hub's `currentAggregationRoot()` reflects the *latest* state, which may have already advanced past the sequence the Verifier is at. To correctly compare, we look up the historical `AggregationRootUpdated` event at the specific `aggSeq` that both sides share, ensuring an apples-to-apples comparison.

#### 3b. Relay Delivery Checks (Verifier → Hub)

These checks verify that transfer roots relayed from each Verifier are actually received by the Hub.

**Comparison logic:**

1. Read `Verifier.latestRelayedIndex()` — how many transfer roots the Verifier has sent.
2. Look up `Hub.eidPosition(eid)` to find the Verifier's slot in the Hub.
3. Read `Hub.transferTreeIndex(position - 1)` — how many transfer roots the Hub has received from this Verifier.
4. If the Verifier has relayed more than the Hub has received: measure the delay.

| Alert | Severity | Condition | What it means |
|---|---|---|---|
| Relay delivery delayed | Warning | `Verifier.latestRelayedIndex() > Hub.transferTreeIndex(position)` AND the `TransferRootRelayed` event for the first missing index is older than `root_delay_threshold_seconds` | Transfer roots relayed by the Verifier are not being delivered to the Hub within the expected timeframe. The L2→L1 bridge message delivery may be stalled. |
| Relay delivery delayed (very old) | Warning | Same as above, but the relay event is not found within the lookback window (500,000 L2 blocks) | The relay has been stalled for a very long time. |

**Why this matters:** Transfer root relay is the mechanism by which L2 transfer activity is communicated back to L1. If relay delivery stalls, the Hub won't have up-to-date information about L2 transfers, which can delay or block crosschain settlements.

---

## Alert System

### Severity Levels

| Severity | Color | Usage |
|---|---|---|
| **Warning** | Yellow (0xFFD700) | Requires attention; system is degraded but not broken |
| **Critical** | Red (0xFF0000) | Immediate action required; potential data inconsistency or service outage |

### Deduplication

Alerts are deduplicated by a key of `{domain}:{severity}:{title}`. If the same alert fires again within the cooldown period (default: 1 hour), it is suppressed. This prevents alert fatigue during prolonged incidents.

### Discord Integration

Alerts are sent as Discord webhook embeds, batched in groups of up to 10 (Discord's per-message limit). Each embed includes:
- Title and description
- Severity and domain fields
- Context-specific fields (chain, indices, delays, etc.)

---

## Stats Reporter

When `stats_interval_seconds > 0`, the watcher periodically sends a comprehensive status report to Discord containing three embeds:

**Balance Stats** — Current ETH balance for every monitored account on every chain.

**Indexer Stats** — Table of on-chain index, `tree_synced_index`, and `latestProvedIndex` for each token on each chain.

**Crosschain Stats** — Per-token breakdown showing Hub `aggSeq`/root status, each Verifier's sync status, and relay delivery status.

---

## Project Structure

```
watcher/
  .env.example          # Template for environment variables
  watcher.yaml          # Main configuration file
  Cargo.toml            # Rust package manifest
  src/
    main.rs             # Entry point, CLI parsing, main loop
    config.rs           # Configuration structures and YAML loading
    alert.rs            # Alert types, deduplication, Discord webhook
    balance.rs          # Balance monitoring domain
    indexer_monitor.rs  # Indexer pipeline staleness detection
    crosschain.rs       # Crosschain root sync and relay monitoring
    stats.rs            # Periodic stats collection and reporting
```
