// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title PoseidonAggregationConfig
/// @notice Configuration constants for Poseidon Merkle tree used in ZK proof aggregation.

/// @dev Height of the Poseidon Merkle tree used for proof aggregation.
uint256 constant POSEIDON_TREE_HEIGHT = 6;

/// @dev Number of precomputed zero hashes needed (one per level plus root).
uint256 constant POSEIDON_ZERO_HASH_COUNT = POSEIDON_TREE_HEIGHT + 1;

/// @dev Maximum number of leaves the tree can hold (2^POSEIDON_TREE_HEIGHT).
uint256 constant POSEIDON_MAX_LEAVES = 2 ** POSEIDON_TREE_HEIGHT;
