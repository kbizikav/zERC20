// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

/* solhint-disable gas-custom-errors */

import {Test} from "forge-std/Test.sol";
import {PoseidonAggregationLib} from "../../src/utils/PoseidonAggregationLib.sol";
import {PoseidonT3} from "poseidon-solidity/contracts/PoseidonT3.sol";
import {
    POSEIDON_TREE_HEIGHT,
    POSEIDON_ZERO_HASH_COUNT,
    POSEIDON_MAX_LEAVES
} from "../../src/utils/PoseidonAggregationConfig.sol";

contract PoseidonAggregationLibHarness {
    function computeAggregationRoot(uint256[] calldata leaves, uint256[POSEIDON_ZERO_HASH_COUNT] calldata zeroHash)
        external
        pure
        returns (uint256)
    {
        uint256[] memory leavesCopy = new uint256[](leaves.length);
        for (uint256 i = 0; i < leaves.length; ++i) {
            leavesCopy[i] = leaves[i];
        }

        uint256[POSEIDON_ZERO_HASH_COUNT] memory zeroHashCopy;
        for (uint256 i = 0; i < POSEIDON_ZERO_HASH_COUNT; ++i) {
            zeroHashCopy[i] = zeroHash[i];
        }

        return PoseidonAggregationLib.computeAggregationRoot(leavesCopy, zeroHashCopy);
    }

    function generateZeroHashes() external pure returns (uint256[POSEIDON_ZERO_HASH_COUNT] memory) {
        return PoseidonAggregationLib.generateZeroHashes();
    }
}

contract PoseidonAggregationLibTest is Test {
    uint256 private constant TREE_HEIGHT = POSEIDON_TREE_HEIGHT;
    uint256 private constant ZERO_HASH_COUNT = POSEIDON_ZERO_HASH_COUNT;
    uint256 private constant MAX_LEAVES = POSEIDON_MAX_LEAVES;

    function testComputeAggregationRootMatchesManual() public pure {
        uint256[ZERO_HASH_COUNT] memory zeroHash = PoseidonAggregationLib.generateZeroHashes();

        _assertMatches(0, zeroHash);
        _assertMatches(1, zeroHash);
        _assertMatches(17, zeroHash);
        _assertMatches(MAX_LEAVES, zeroHash);
    }

    function testComputeAggregationRootMutatesLeavesInPlace() public pure {
        uint256[ZERO_HASH_COUNT] memory zeroHash = PoseidonAggregationLib.generateZeroHashes();
        uint256[] memory leaves = new uint256[](2);
        leaves[0] = 111;
        leaves[1] = 222;

        uint256 left = leaves[0];
        uint256 right = leaves[1];

        PoseidonAggregationLib.computeAggregationRoot(leaves, zeroHash);

        uint256[2] memory inputs;
        inputs[0] = left;
        inputs[1] = right;
        uint256 expectedLevel0 = PoseidonT3.hash(inputs);

        require(leaves[0] == expectedLevel0, "leaves[0] not overwritten");
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

    // ==================== generateZeroHashes Tests ====================

    function testGenerateZeroHashesFirstElementIsZero() public pure {
        uint256[ZERO_HASH_COUNT] memory zeroHash = PoseidonAggregationLib.generateZeroHashes();

        assertEq(zeroHash[0], 0, "zeroHash[0] should be 0");
    }

    function testGenerateZeroHashesIsConsistent() public pure {
        uint256[ZERO_HASH_COUNT] memory zeroHash1 = PoseidonAggregationLib.generateZeroHashes();
        uint256[ZERO_HASH_COUNT] memory zeroHash2 = PoseidonAggregationLib.generateZeroHashes();

        for (uint256 i = 0; i < ZERO_HASH_COUNT; ++i) {
            assertEq(zeroHash1[i], zeroHash2[i], "zero hashes should be deterministic");
        }
    }

    function testGenerateZeroHashesChainCorrectly() public pure {
        uint256[ZERO_HASH_COUNT] memory zeroHash = PoseidonAggregationLib.generateZeroHashes();

        for (uint256 i = 1; i < ZERO_HASH_COUNT; ++i) {
            uint256 expected = _hashPair(zeroHash[i - 1], zeroHash[i - 1]);
            assertEq(zeroHash[i], expected, "zeroHash[i] should be hash of previous level");
        }
    }

    // ==================== computeAggregationRoot Edge Cases ====================

    function testComputeAggregationRootWithEmptyLeaves() public pure {
        uint256[ZERO_HASH_COUNT] memory zeroHash = PoseidonAggregationLib.generateZeroHashes();
        uint256[] memory leaves = new uint256[](0);

        uint256 root = PoseidonAggregationLib.computeAggregationRoot(leaves, zeroHash);

        assertEq(root, zeroHash[TREE_HEIGHT], "empty leaves should return zeroHash[TREE_HEIGHT]");
    }

    function testComputeAggregationRootWithSingleLeaf() public pure {
        uint256[ZERO_HASH_COUNT] memory zeroHash = PoseidonAggregationLib.generateZeroHashes();
        uint256[] memory leaves = new uint256[](1);
        leaves[0] = 12345;

        uint256[] memory leavesCopy = new uint256[](1);
        leavesCopy[0] = leaves[0];

        uint256 root = PoseidonAggregationLib.computeAggregationRoot(leaves, zeroHash);
        uint256 expected = _manualAggregationRoot(leavesCopy);

        assertEq(root, expected, "single leaf root mismatch");
    }

    function testComputeAggregationRootWithTwoLeaves() public pure {
        uint256[ZERO_HASH_COUNT] memory zeroHash = PoseidonAggregationLib.generateZeroHashes();
        uint256[] memory leaves = new uint256[](2);
        leaves[0] = 111;
        leaves[1] = 222;

        uint256[] memory leavesCopy = new uint256[](2);
        leavesCopy[0] = leaves[0];
        leavesCopy[1] = leaves[1];

        uint256 root = PoseidonAggregationLib.computeAggregationRoot(leaves, zeroHash);
        uint256 expected = _manualAggregationRoot(leavesCopy);

        assertEq(root, expected, "two leaves root mismatch");
    }

    function testComputeAggregationRootWithOddLeaves() public pure {
        uint256[ZERO_HASH_COUNT] memory zeroHash = PoseidonAggregationLib.generateZeroHashes();
        uint256[] memory leaves = new uint256[](3);
        leaves[0] = 100;
        leaves[1] = 200;
        leaves[2] = 300;

        uint256[] memory leavesCopy = new uint256[](3);
        leavesCopy[0] = leaves[0];
        leavesCopy[1] = leaves[1];
        leavesCopy[2] = leaves[2];

        uint256 root = PoseidonAggregationLib.computeAggregationRoot(leaves, zeroHash);
        uint256 expected = _manualAggregationRoot(leavesCopy);

        assertEq(root, expected, "odd leaves root mismatch");
    }

    function testComputeAggregationRootWithMaxLeaves() public pure {
        uint256[ZERO_HASH_COUNT] memory zeroHash = PoseidonAggregationLib.generateZeroHashes();
        uint256[] memory leaves = new uint256[](MAX_LEAVES);
        for (uint256 i = 0; i < MAX_LEAVES; ++i) {
            leaves[i] = i + 1;
        }

        uint256 root = PoseidonAggregationLib.computeAggregationRoot(leaves, zeroHash);

        // Should not revert and return a valid root
        assertGt(root, 0, "max leaves should produce non-zero root");
    }

    function testComputeAggregationRootRevertsOnTooManyLeaves() public {
        PoseidonAggregationLibHarness harness = new PoseidonAggregationLibHarness();
        uint256[ZERO_HASH_COUNT] memory zeroHash = harness.generateZeroHashes();
        uint256[] memory leaves = new uint256[](MAX_LEAVES + 1);

        vm.expectRevert(
            abi.encodeWithSelector(PoseidonAggregationLib.TooManyLeaves.selector, MAX_LEAVES + 1, MAX_LEAVES)
        );
        harness.computeAggregationRoot(leaves, zeroHash);
    }

    // ==================== Determinism Tests ====================

    function testComputeAggregationRootIsDeterministic() public pure {
        uint256[ZERO_HASH_COUNT] memory zeroHash = PoseidonAggregationLib.generateZeroHashes();
        uint256[] memory leaves1 = new uint256[](5);
        uint256[] memory leaves2 = new uint256[](5);
        for (uint256 i = 0; i < 5; ++i) {
            leaves1[i] = i * 100;
            leaves2[i] = i * 100;
        }

        uint256 root1 = PoseidonAggregationLib.computeAggregationRoot(leaves1, zeroHash);
        uint256 root2 = PoseidonAggregationLib.computeAggregationRoot(leaves2, zeroHash);

        assertEq(root1, root2, "same inputs should produce same root");
    }

    function testComputeAggregationRootDiffersForDifferentLeaves() public pure {
        uint256[ZERO_HASH_COUNT] memory zeroHash = PoseidonAggregationLib.generateZeroHashes();
        uint256[] memory leaves1 = new uint256[](2);
        uint256[] memory leaves2 = new uint256[](2);
        leaves1[0] = 111;
        leaves1[1] = 222;
        leaves2[0] = 111;
        leaves2[1] = 333; // different

        uint256 root1 = PoseidonAggregationLib.computeAggregationRoot(leaves1, zeroHash);
        uint256 root2 = PoseidonAggregationLib.computeAggregationRoot(leaves2, zeroHash);

        assertTrue(root1 != root2, "different inputs should produce different roots");
    }
}
