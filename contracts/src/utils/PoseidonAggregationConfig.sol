// SPDX-License-Identifier: Unlicense
pragma solidity ^0.8.20;

uint256 constant POSEIDON_TREE_HEIGHT = 6;
uint256 constant POSEIDON_ZERO_HASH_COUNT = POSEIDON_TREE_HEIGHT + 1;
uint256 constant POSEIDON_MAX_LEAVES = 2 ** POSEIDON_TREE_HEIGHT;
