# Integration Guide

This guide covers how to integrate zERC20 into your application.

## Overview

This guide covers three tips for integrating zERC20:

1. **Token Integration**: Use zERC20 as a standard ERC-20 token in your DeFi protocols, wallets, or dApps
2. **Oracle Integration**: Leverage zERC20's Transfer Merkle Tree as an on-chain oracle to verify transfer history with ZK proofs
3. **Self-Hosted Indexer**: Run your own indexer node for maximum privacy, avoiding sender-recipient linkage exposure

## Token Integration

zERC20 is fully ERC-20 compatible. Any application that supports ERC-20 tokens can use zERC20.

### Contract Addresses

See [Contract Addresses](../reference/addresses.md) for deployment addresses on each chain.

### Standard ERC-20 Interface

```javascript
interface IERC20 {
    function balanceOf(address account) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
    function approve(address spender, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
}
```

### Wrapping and Unwrapping

Use the `LiquidityManager` contract to convert between the underlying token and zERC20.

```javascript
interface ILiquidityManager {
    /// @notice Wrap underlying tokens to receive zERC20
    /// @param amount Amount of underlying tokens to wrap
    /// @param receiver Address to receive the zERC20 tokens
    /// @return amountOut Amount of zERC20 tokens received (includes reward)
    function wrap(uint256 amount, address receiver) external payable returns (uint256 amountOut);

    /// @notice Wrap with slippage protection
    /// @param amount Amount of underlying tokens to wrap
    /// @param minOut Minimum zERC20 tokens to receive (reverts if less)
    /// @param receiver Address to receive the zERC20 tokens
    /// @return amountOut Amount of zERC20 tokens received
    function wrapWithMinOut(uint256 amount, uint256 minOut, address receiver)
        external
        payable
        returns (uint256 amountOut);

    /// @notice Unwrap zERC20 to receive underlying tokens
    /// @param amount Amount of zERC20 tokens to unwrap
    /// @param receiver Address to receive the underlying tokens
    /// @return amountOut Amount of underlying tokens received (minus fee)
    function unwrap(uint256 amount, address receiver) external returns (uint256 amountOut);

    /// @notice Unwrap with slippage protection
    /// @param amount Amount of zERC20 tokens to unwrap
    /// @param minOut Minimum underlying tokens to receive (reverts if less)
    /// @param receiver Address to receive the underlying tokens
    /// @return amountOut Amount of underlying tokens received
    function unwrapWithMinOut(uint256 amount, uint256 minOut, address receiver)
        external
        returns (uint256 amountOut);

    /// @notice Get the reward amount for wrapping
    function quoteWrapReward(uint256 amount) external view returns (uint256 rewardAmount);

    /// @notice Get the fee amount for unwrapping
    function quoteUnwrapFee(uint256 amount) external view returns (uint256 feeAmount);
}
```

**Example Usage:**

```javascript
// Wrap: Convert 100 USDC to zUSDC
IERC20(usdc).approve(address(liquidityManager), 100e6);
uint256 zAmount = liquidityManager.wrap(100e6, msg.sender);

// Unwrap: Convert 100 zUSDC back to USDC
uint256 usdcAmount = liquidityManager.unwrap(100e6, msg.sender);
```

See [Fees and Rewards](../users/fees-and-rewards.md) for details on wrap rewards and unwrap fees.

## zERC20 as Oracle

zERC20 maintains a complete history of all transfers as a Merkle tree, using a ZK-friendly Poseidon hash function. External developers can leverage this Transfer Merkle Tree as an on-chain oracle for transfer history verification.

### Leaf Structure

Each leaf in the Transfer Merkle Tree represents a single transfer:

```
leaf_hash = Poseidon3(from, to, value)
```

| Field   | Type      | Description                                    |
| ------- | --------- | ---------------------------------------------- |
| `from`  | `address` | Sender address (converted to field element)    |
| `to`    | `address` | Recipient address (converted to field element) |
| `value` | `uint256` | Transfer amount (converted to field element)   |

### Tree Structure

There are two types of Merkle roots:

| Root Type                | Description                        | Tree Height |
| ------------------------ | ---------------------------------- | ----------- |
| **Local Transfer Root**  | Per-chain transfer Merkle root     | 40          |
| **Global Transfer Root** | Cross-chain aggregated Merkle root | 46 (40 + 6) |

The Global Transfer Tree is constructed by aggregating Local Transfer Roots from all chains using an Aggregation Tree (height 6, supporting up to 64 chains).

```
Global Transfer Tree Structure:

         [Global Root]
              │
    ┌─────────┴─────────┐
    │  Aggregation Tree │  (height: 6)
    │   (up to 64 chains)│
    └─────────┬─────────┘
              │
   ┌──────────┼──────────┐
   │          │          │
[Chain 0]  [Chain 1]  [Chain N]
   │          │          │
[Local     [Local     [Local
 Transfer   Transfer   Transfer
 Root]      Root]      Root]
   │          │          │
[Transfer  [Transfer  [Transfer
 Tree]      Tree]      Tree]
 (h:40)     (h:40)     (h:40)
```

### Reading Merkle Roots from Contract

External contracts can query the proven Merkle roots from the Verifier contract:

```javascript
interface IVerifier {
    /// @notice Get the local transfer root for a given index
    /// @param index The tree index (increments with each proof)
    /// @return The local transfer Merkle root
    function provedTransferRoots(uint64 index) external view returns (uint256);

    /// @notice Get the global transfer root for a given index
    /// @param index The aggregation sequence number
    /// @return The global transfer Merkle root
    function globalTransferRoots(uint64 index) external view returns (uint256);
}
```

**Example Usage:**

Merkle proof verification can be performed either **on-chain in a smart contract** or **off-chain using ZKP circuits**. Choose the approach that best fits your use case:

- **On-chain verification**: Suitable for simple membership proofs where gas costs are acceptable
- **ZKP verification**: Ideal for privacy-preserving applications or complex logic that would be expensive on-chain

```javascript
// On-chain verification example
contract MyContract {
    IVerifier public verifier;

    function verifyLocalTransfer(
        uint64 treeIndex,
        bytes32[] calldata siblings,
        uint64 leafIndex,
        address from,
        address to,
        uint256 value
    ) external view returns (bool) {
        uint256 expectedRoot = verifier.provedTransferRoots(treeIndex);
        // Verify Merkle proof against expectedRoot using Poseidon hash
        // ...
    }
}
```

### Poseidon Hash Compatibility

The Poseidon hash used in zERC20 is fully compatible with [circomlib's Poseidon library](https://github.com/iden3/circomlib/blob/master/circuits/poseidon.circom):

| Usage     | circomlib Template | Description                 |
| --------- | ------------------ | --------------------------- |
| Leaf hash | `Poseidon(3)`      | `Poseidon(from, to, value)` |
| Node hash | `Poseidon(2)`      | `Poseidon(left, right)`     |

This compatibility allows developers to build custom ZK circuits using circomlib that can verify membership proofs against zERC20's Transfer Merkle Tree.

### Obtaining Merkle Proofs

#### Local Transfer Merkle Proof

Query the indexer node to obtain Local Transfer Merkle proofs:

**Endpoint:** `POST /proofs`

**Request:**

```json
{
  "chain_id": 1,
  "token_address": "0x...",
  "target_index": 100,
  "leaf_indices": [42, 43, 44]
}
```

**Response:**

```json
[
  {
    "target_index": 100,
    "leaf_index": 42,
    "root": "0x...",
    "hash_chain": "0x...",
    "siblings": ["0x...", "0x...", ...]
  }
]
```

| Field          | Description                                   |
| -------------- | --------------------------------------------- |
| `target_index` | The tree index (snapshot) to prove against    |
| `leaf_index`   | The position of the leaf in the tree          |
| `root`         | The Merkle root at the target index           |
| `hash_chain`   | The hash chain value at the target index      |
| `siblings`     | Array of 40 sibling hashes for the proof path |

**Supporting Endpoint - Get Tree Index:**

`GET /tree-index?chain_id={chainId}&token_address={address}&transfer_root={root}`

Returns the tree index for a given Merkle root.

#### Global Transfer Merkle Proof

Global Merkle proofs are constructed by concatenating:

1. **Local Transfer Merkle Proof** (40 siblings)
2. **Aggregation Tree Proof** (6 siblings)

**Construction Algorithm:**

```
1. Fetch local_merkle_proof from indexer (40 siblings)
2. Determine aggregation_index from chain_id position in Hub contract
3. Compute aggregation_merkle_proof from Hub's AggregationRootUpdated event
4. Concatenate: global_proof = local_proof ++ aggregation_proof (46 siblings)
5. Compute global_leaf_index:
   global_leaf_index = (aggregation_index << 40) + local_leaf_index
```

**Aggregation State from Hub Contract:**

The `AggregationRootUpdated` event from the Hub contract provides the snapshot of all local roots:

```javascript
event AggregationRootUpdated(
    uint256 indexed root,
    uint64 indexed aggSeq,
    uint256[] transferRootsSnapshot,
    uint64[] transferTreeIndicesSnapshot
);
```

## Running Your Own Indexer

For maximum privacy, run your own indexer instance to avoid sender-recipient linkage leaks.

### Docker Compose

```bash
git clone https://github.com/kbizikav/zERC20.git
cd zERC20
docker compose up -d
```

### Configuration

Set environment variables:

```bash
DATABASE_URL=postgres://...
RPC_URL=https://...
TOKENS_FILE_PATH=./config/tokens.json
```

See the Docker Compose configuration above for setup instructions.
