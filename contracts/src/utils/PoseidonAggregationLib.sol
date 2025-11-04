// SPDX-License-Identifier: Unlicense
pragma solidity ^0.8.20;

import {PoseidonT3} from "poseidon-solidity/contracts/PoseidonT3.sol";
import {POSEIDON_TREE_HEIGHT, POSEIDON_ZERO_HASH_COUNT, POSEIDON_MAX_LEAVES} from "./PoseidonAggregationConfig.sol";

/**
 * @title PoseidonAggregationLib
 * @notice Utility helpers for building Poseidon-based binary aggregation trees.
 */
library PoseidonAggregationLib {
    uint256 constant TREE_HEIGHT = POSEIDON_TREE_HEIGHT;
    uint256 constant ZERO_HASH_COUNT = POSEIDON_ZERO_HASH_COUNT;
    uint256 constant MAX_LEAVES = POSEIDON_MAX_LEAVES;

    /**
     * @notice Computes the aggregation root for the provided leaves, padding with zeros up to TREE_HEIGHT.
     * @param leaves The list of active leaves.
     * @param zeroHash Pre-computed zero hashes for depths 0..TREE_HEIGHT.
     */
    function computeAggregationRoot(uint256[] memory leaves, uint256[ZERO_HASH_COUNT] memory zeroHash)
        internal
        pure
        returns (uint256)
    {
        uint256 count = leaves.length;
        if (count == 0) {
            return zeroHash[TREE_HEIGHT];
        }

        uint256 depth;
        while (count > 1) {
            uint256 nextCount = (count + 1) >> 1;
            for (uint256 i = 0; i < nextCount; ++i) {
                uint256 left = leaves[2 * i];
                uint256 right = (2 * i + 1 < count) ? leaves[2 * i + 1] : zeroHash[depth];
                leaves[i] = _hashPair(left, right);
            }
            count = nextCount;
            unchecked {
                ++depth;
            }
        }

        uint256 root = leaves[0];
        for (uint256 d = depth; d < TREE_HEIGHT; ++d) {
            root = _hashPair(root, zeroHash[d]);
        }
        return root;
    }

    /**
     * @notice Generates the zero hash table for depths 0..TREE_HEIGHT.
     */
    function generateZeroHashes() internal pure returns (uint256[ZERO_HASH_COUNT] memory zeroHash) {
        zeroHash[0] = 0;
        for (uint256 i = 1; i <= TREE_HEIGHT; ++i) {
            zeroHash[i] = _hashPair(zeroHash[i - 1], zeroHash[i - 1]);
        }
    }

    /**
     * @notice Hashes a pair of leaves using PoseidonT3.
     */
    function _hashPair(uint256 left, uint256 right) internal pure returns (uint256) {
        uint256[2] memory inputs;
        inputs[0] = left;
        inputs[1] = right;
        return PoseidonT3.hash(inputs);
    }
}
