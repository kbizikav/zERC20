// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {PoseidonAggregationLib} from "../../src/utils/PoseidonAggregationLib.sol";
import {PoseidonT3} from "poseidon-solidity/contracts/PoseidonT3.sol";
import {
    POSEIDON_TREE_HEIGHT,
    POSEIDON_ZERO_HASH_COUNT,
    POSEIDON_MAX_LEAVES
} from "../../src/utils/PoseidonAggregationConfig.sol";

contract PoseidonAggregationLibTest is Test {
    uint256 constant TREE_HEIGHT = POSEIDON_TREE_HEIGHT;
    uint256 constant ZERO_HASH_COUNT = POSEIDON_ZERO_HASH_COUNT;
    uint256 constant MAX_LEAVES = POSEIDON_MAX_LEAVES;

    function testComputeAggregationRootMatchesManual() public pure {
        uint256[ZERO_HASH_COUNT] memory zeroHash = PoseidonAggregationLib.generateZeroHashes();

        _assertMatches(0, zeroHash);
        _assertMatches(1, zeroHash);
        _assertMatches(17, zeroHash);
        _assertMatches(MAX_LEAVES, zeroHash);
    }

    function _assertMatches(uint256 count, uint256[ZERO_HASH_COUNT] memory zeroHash) internal pure {
        uint256[] memory leaves = new uint256[](count);
        for (uint256 i = 0; i < count; ++i) {
            leaves[i] = uint256(keccak256(abi.encodePacked(i + 1)));
        }

        uint256 expected = _manualAggregationRoot(leaves);
        uint256 actual = PoseidonAggregationLib.computeAggregationRoot(leaves, zeroHash);
        require(actual == expected, "root mismatch");
    }

    function _manualAggregationRoot(uint256[] memory leaves) internal pure returns (uint256) {
        uint256 width = MAX_LEAVES;
        uint256[] memory level = new uint256[](width);

        uint256 count = leaves.length;
        for (uint256 i = 0; i < count; ++i) {
            level[i] = leaves[i];
        }
        for (uint256 i = count; i < width; ++i) {
            level[i] = 0;
        }

        while (width > 1) {
            uint256 nextWidth = width >> 1;
            for (uint256 i = 0; i < nextWidth; ++i) {
                level[i] = _hashPair(level[2 * i], level[2 * i + 1]);
            }
            width = nextWidth;
        }

        return level[0];
    }

    function _hashPair(uint256 left, uint256 right) internal pure returns (uint256) {
        uint256[2] memory inputs;
        inputs[0] = left;
        inputs[1] = right;
        return PoseidonT3.hash(inputs);
    }
}
