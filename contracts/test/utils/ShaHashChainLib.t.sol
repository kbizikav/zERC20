// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

/* solhint-disable gas-custom-errors */

import {Test} from "forge-std/Test.sol";
import {ShaHashChainLib} from "../../src/utils/ShaHashChainLib.sol";

contract ShaHashChainLibTest is Test {
    uint256 private constant ZERO_VECTOR_EXPECTED = 0x00f37f8d1931b3bdf767e7510dd69509fbf23af1f7654933d0a4d291cbdd4418;
    uint256 private constant LOWER_248_BITS_MASK = type(uint256).max >> 8;

    // ==================== Basic Functionality Tests ====================

    function testComputeZeroVectorMatchesReference() public pure {
        uint256 actual = ShaHashChainLib.compute(0, address(0), address(0), 0);
        assertEq(actual, ZERO_VECTOR_EXPECTED, "zero vector hash mismatch");
    }

    function testComputeWithNonZeroInputs() public pure {
        address from = address(0x1234567890AbcdEF1234567890aBcdef12345678);
        address to = address(0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF);
        uint256 prev = 12345;
        uint256 value = 1 ether;

        uint256 result = ShaHashChainLib.compute(prev, from, to, value);

        // Result should be non-zero and within 248 bits
        assertGt(result, 0, "result should be non-zero");
        assertEq(result & ~LOWER_248_BITS_MASK, 0, "upper 8 bits should be zero");
    }

    // ==================== 248-bit Mask Tests ====================

    function testComputeAlwaysReturns248Bits() public pure {
        // Test with max values that would produce high bits in sha256
        uint256 result = ShaHashChainLib.compute(
            type(uint256).max, address(type(uint160).max), address(type(uint160).max), type(uint256).max
        );

        assertEq(result & ~LOWER_248_BITS_MASK, 0, "upper 8 bits must be zero");
    }

    function testComputePreserves248Bits() public pure {
        uint256 result = ShaHashChainLib.compute(0, address(0), address(0), 0);

        // Verify the result matches expected (which has upper byte as 0x00)
        assertEq(result, ZERO_VECTOR_EXPECTED, "should preserve lower 248 bits");
        assertEq(result >> 248, 0, "upper byte should be zero");
    }

    // ==================== Determinism Tests ====================

    function testComputeIsDeterministic() public pure {
        address from = address(0x1111);
        address to = address(0x2222);
        uint256 prev = 100;
        uint256 value = 500;

        uint256 result1 = ShaHashChainLib.compute(prev, from, to, value);
        uint256 result2 = ShaHashChainLib.compute(prev, from, to, value);

        assertEq(result1, result2, "same inputs should produce same output");
    }

    function testComputeDiffersForDifferentPrev() public pure {
        address from = address(0x1111);
        address to = address(0x2222);
        uint256 value = 500;

        uint256 result1 = ShaHashChainLib.compute(100, from, to, value);
        uint256 result2 = ShaHashChainLib.compute(101, from, to, value);

        assertTrue(result1 != result2, "different prev should produce different output");
    }

    function testComputeDiffersForDifferentFrom() public pure {
        address to = address(0x2222);
        uint256 prev = 100;
        uint256 value = 500;

        uint256 result1 = ShaHashChainLib.compute(prev, address(0x1111), to, value);
        uint256 result2 = ShaHashChainLib.compute(prev, address(0x3333), to, value);

        assertTrue(result1 != result2, "different from should produce different output");
    }

    function testComputeDiffersForDifferentTo() public pure {
        address from = address(0x1111);
        uint256 prev = 100;
        uint256 value = 500;

        uint256 result1 = ShaHashChainLib.compute(prev, from, address(0x2222), value);
        uint256 result2 = ShaHashChainLib.compute(prev, from, address(0x4444), value);

        assertTrue(result1 != result2, "different to should produce different output");
    }

    function testComputeDiffersForDifferentValue() public pure {
        address from = address(0x1111);
        address to = address(0x2222);
        uint256 prev = 100;

        uint256 result1 = ShaHashChainLib.compute(prev, from, to, 500);
        uint256 result2 = ShaHashChainLib.compute(prev, from, to, 600);

        assertTrue(result1 != result2, "different value should produce different output");
    }

    // ==================== Chaining Tests ====================

    function testComputeChaining() public pure {
        address alice = address(0xA11CE);
        address bob = address(0xB0B);
        address charlie = address(0xC4A1E);

        // Chain: 0 -> alice sends 100 to bob -> bob sends 50 to charlie
        uint256 hash1 = ShaHashChainLib.compute(0, alice, bob, 100);
        uint256 hash2 = ShaHashChainLib.compute(hash1, bob, charlie, 50);

        // Verify chain produces valid results
        assertGt(hash1, 0, "first hash should be non-zero");
        assertGt(hash2, 0, "second hash should be non-zero");
        assertTrue(hash1 != hash2, "chained hashes should differ");
    }

    function testComputeChainOrderMatters() public pure {
        address alice = address(0xA11CE);
        address bob = address(0xB0B);

        // Forward chain
        uint256 forward1 = ShaHashChainLib.compute(0, alice, bob, 100);
        uint256 forward2 = ShaHashChainLib.compute(forward1, bob, alice, 100);

        // Reverse chain
        uint256 reverse1 = ShaHashChainLib.compute(0, bob, alice, 100);
        uint256 reverse2 = ShaHashChainLib.compute(reverse1, alice, bob, 100);

        assertTrue(forward2 != reverse2, "different chain order should produce different final hash");
    }

    // ==================== Edge Cases ====================

    function testComputeWithMaxPrev() public pure {
        uint256 result = ShaHashChainLib.compute(type(uint256).max, address(0), address(0), 0);

        assertEq(result & ~LOWER_248_BITS_MASK, 0, "upper 8 bits should be zero");
        assertTrue(result != ZERO_VECTOR_EXPECTED, "max prev should differ from zero prev");
    }

    function testComputeWithMaxValue() public pure {
        uint256 result = ShaHashChainLib.compute(0, address(0), address(0), type(uint256).max);

        assertEq(result & ~LOWER_248_BITS_MASK, 0, "upper 8 bits should be zero");
        assertTrue(result != ZERO_VECTOR_EXPECTED, "max value should differ from zero value");
    }

    function testComputeWithSameFromAndTo() public pure {
        address addr = address(0x1234);

        uint256 result = ShaHashChainLib.compute(0, addr, addr, 100);

        assertGt(result, 0, "self-transfer should produce valid hash");
        assertEq(result & ~LOWER_248_BITS_MASK, 0, "upper 8 bits should be zero");
    }

    // ==================== Fuzz Tests ====================

    function testFuzzComputeAlwaysReturns248Bits(uint256 prev, address from, address to, uint256 value) public pure {
        uint256 result = ShaHashChainLib.compute(prev, from, to, value);

        assertEq(result & ~LOWER_248_BITS_MASK, 0, "upper 8 bits must always be zero");
    }

    function testFuzzComputeDeterminism(uint256 prev, address from, address to, uint256 value) public pure {
        uint256 result1 = ShaHashChainLib.compute(prev, from, to, value);
        uint256 result2 = ShaHashChainLib.compute(prev, from, to, value);

        assertEq(result1, result2, "same inputs must produce same output");
    }
}
