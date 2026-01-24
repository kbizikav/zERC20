# Contract Specification

## Overview

The zERC20 contract system consists of:

- **zERC20**: Privacy-enabled ERC-20 token
- **Verifier**: Proof verification and teleport execution
- **Hub**: Cross-chain root aggregation
- **LiquidityManager**: Liquidity entry/exit policy
- **Adaptor**: Cross-chain exit via Stargate
- **Fee Manager** (off-chain): Dynamic liquidity target adjustment

## zERC20

**Location**: `contracts/src/zERC20.sol`

An upgradeable ERC-20 that tracks all transfers in a hash chain for ZKP verification.

### Key Features

- Emits `IndexedTransfer(index, from, to, value)` for every transfer
- Maintains truncated SHA-256 hash chain: `hashChain = SHA256(hashChain || to || value)[0:248]`
- Exposes `teleport` for Verifier-initiated mints

### Functions

```solidity
// Called by Verifier after successful proof verification
function teleport(address to, uint256 value) external onlyVerifier;

// Called by LiquidityManager for liquidity operations
function mint(address to, uint256 amount) external onlyLiquidityAuthority;
function burn(address from, uint256 amount) external onlyLiquidityAuthority;
```

### Events

```solidity
event IndexedTransfer(uint256 indexed index, address indexed from, address indexed to, uint256 value);
event Teleport(address indexed to, uint256 value);
```

### Constraints

- All transfer values must be ≤ 2^248 - 1 (fits in BN254 scalar field)
- Reverts with `ValueTooLarge` if exceeded

## Verifier

**Location**: `contracts/src/Verifier.sol`

LayerZero OApp that verifies ZK proofs and manages teleports.

### Key Features

1. Records hash chain checkpoints for proof anchoring
2. Verifies Nova proofs for transfer root transitions
3. Verifies Nova/Groth16 proofs for withdrawals
4. Relays roots to Hub via LayerZero

### Functions

```solidity
// Reserve a hash chain checkpoint for proof anchoring
function reserveHashChain(uint256 index) external;

// Prove a new transfer root (called by indexer)
function proveTransferRoot(bytes calldata proof, ...) external;

// Batch withdrawal with Nova proof
function teleport(
    bytes calldata proof,
    GeneralRecipient calldata gr,
    uint256 rootHint,
    bool isGlobal,
    ...
) external;

// Single withdrawal with Groth16 proof
function singleTeleport(
    bytes calldata proof,
    GeneralRecipient calldata gr,
    uint256 rootHint,
    bool isGlobal,
    ...
) external;

// Relay local root to Hub
function relayTransferRoot() external payable;
```

### State

```solidity
mapping(uint256 => bytes32) public reservedHashChains;     // index → hashChain snapshot
mapping(uint256 => bytes32) public provedTransferRoots;    // index → merkle root
mapping(bytes32 => uint256) public totalTeleported;        // recipientHash → total minted
mapping(uint256 => bytes32) public globalTransferRoots;    // aggSeq → global root
```

### GeneralRecipient

Binds withdrawals to a specific destination:

```solidity
struct GeneralRecipient {
    uint256 chainId;
    address addr;
    uint256 tweak;
}
// Hash: Poseidon(chainId, addr, tweak)
```

### Emergency Handling

If a proof inconsistency is detected (mismatched roots for same index), the contract pauses automatically. Owner must rotate verifiers and call `deactivateEmergency` to resume.

## Hub

**Location**: `contracts/src/Hub.sol`

Central aggregator for cross-chain transfer roots.

### Key Features

- Receives roots from all chain Verifiers via LayerZero
- Aggregates into Poseidon tree (max 64 leaves)
- Broadcasts global root to all Verifiers

### Functions

```solidity
// Register a new token (owner only)
function registerToken(
    uint256 chainId,
    uint32 eid,
    address verifier,
    address token
) external onlyOwner;

// Broadcast current aggregation to all chains
function broadcast(uint32[] calldata targetEids) external payable;

// Estimate broadcast fee
function quoteBroadcast(uint32[] calldata targetEids) external view returns (uint256);
```

### Events

```solidity
event AggregationRootUpdated(
    uint256 aggSeq,
    bytes32 globalRoot,
    bytes32[] leaves,
    uint256[] treeIndices
);
```

## LiquidityManager

**Location**: `contracts/src/liquidity/LiquidityManager.sol`

Manages liquidity entry/exit with incentive curves.

### Key Features

- Wraps underlying tokens into zERC20
- Unwraps zERC20 back to underlying
- Applies incentive fees based on liquidity target

### Functions

```solidity
// Wrap underlying → zERC20
function wrap(address receiver, uint256 amount) external returns (uint256 reward);

// Unwrap zERC20 → underlying
function unwrap(address receiver, uint256 amount) external returns (uint256 fee);
```

### Incentive Curve

Linear incentive density: `density(x) = k * (1 - x / T)` for `x < T`

- Below target: wraps earn rewards, unwraps pay fees
- Above target: no rewards or fees
- Fees accumulate in `feeSurplus` for future incentives

## Fee Manager

**Location**: `fee-manager/` (off-chain service)

Periodic maintenance worker that dynamically adjusts `targetLiquidity` on all `LiquidityManager` contracts across chains.

### Purpose

The Fee Manager ensures balanced liquidity distribution across all chains by automatically updating fee parameters:

```
targetLiquidity = total_underlying_balance / number_of_chains
```

This eliminates manual parameter tuning and automatically adjusts incentives as liquidity flows between chains.

### How It Works

1. **Fetch Balances**: Query underlying token balance (`balance - feeSurplus`) from each LiquidityManager
2. **Calculate Target**: Compute `targetLiquidity = total / chain_count`
3. **Update Parameters**: Call `setFeeParams({ targetLiquidity, k })` on each LiquidityManager
4. **Repeat**: Sleep for configured interval (default: 1 hour)

### FeeParams Structure

```solidity
struct FeeParams {
    uint256 targetLiquidity;  // Liquidity level where incentives fade to zero
    uint256 k;                // Incentive strength coefficient (basis points)
}
```

- `targetLiquidity`: The liquidity level at which wrap rewards and unwrap fees approach zero
- `k`: Controls incentive curve steepness (higher = stronger incentives when liquidity deviates from target)

### Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `FEE_MANAGER_PRIVATE_KEY` | — | Private key with `FEE_MANAGER_ROLE` on each LiquidityManager |
| `FEE_MANAGER_INTERVAL_SECS` | `3600` | Interval between updates (seconds) |
| `FEE_MANAGER_K_BPS` | `1000` | Incentive coefficient k in basis points (1000 = 10%) |

### Native Token Support

The Fee Manager automatically detects whether a LiquidityManager uses native ETH or ERC20:
- **Native ETH**: Detected via ERC-7528 sentinel address `0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE`
- **ERC20**: Balance fetched via `balanceOf()` on underlying token

### Permissions

The Fee Manager requires `FEE_MANAGER_ROLE` on each LiquidityManager:

```solidity
liquidityManager.grantRole(FEE_MANAGER_ROLE, feeManagerAddress);
```

## Key Flows

### 1. Transfer Root Proving

```
1. zERC20 transfer emits IndexedTransfer, updates hashChain
2. Indexer syncs events, builds Merkle tree
3. Indexer calls Verifier.reserveHashChain(index)
4. Indexer generates Nova proof for root transition
5. Indexer calls Verifier.proveTransferRoot()
6. Verifier stores new root in provedTransferRoots
```

### 2. Local Teleport

```
1. User sends zERC20 to burn address
2. Recipient generates ZKP proving burn address ownership
3. Recipient calls Verifier.teleport() or singleTeleport()
4. Verifier validates proof against provedTransferRoots
5. Verifier calls zERC20.teleport() to mint
```

### 3. Global Teleport (Cross-chain)

```
1. Verifier.relayTransferRoot() sends local root to Hub
2. Hub.broadcast() aggregates and sends global root
3. Recipient generates ZKP with isGlobal=true
4. Verifier validates against globalTransferRoots
5. Verifier mints via zERC20.teleport()
```

## Security Notes

- **Value range**: All values checked against 248-bit limit
- **Double-spend prevention**: `totalTeleported` tracks cumulative mints per recipient
- **LayerZero security**: Only accepts messages from known endpoints
- **Upgrade safety**: UUPS pattern with owner-only upgrade
