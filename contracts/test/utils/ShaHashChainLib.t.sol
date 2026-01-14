// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Test} from "forge-std/Test.sol";
import {ShaHashChainLib} from "../../src/utils/ShaHashChainLib.sol";

contract ShaHashChainLibTest is Test {
    uint256 private constant ZERO_VECTOR_EXPECTED = 0x00f37f8d1931b3bdf767e7510dd69509fbf23af1f7654933d0a4d291cbdd4418;

    function testComputeZeroVectorMatchesReference() public pure {
        uint256 actual = ShaHashChainLib.compute(0, address(0), address(0), 0);
        assertEq(actual, ZERO_VECTOR_EXPECTED, "zero vector hash mismatch");
    }
}
