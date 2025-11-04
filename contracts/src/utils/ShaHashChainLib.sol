// SPDX-License-Identifier: Unlicense
pragma solidity ^0.8.20;

library ShaHashChainLib {
    /// @dev Computes sha256( BE(prev,32) || BE(addr,20) || BE(value,32) )
    ///      and returns the lower 248 bits (most-significant byte dropped).
    function compute(uint256 prev, address addr, uint256 value) internal pure returns (uint256 next) {
        next = uint256(sha256(abi.encodePacked(prev, addr, value)))
            & 0x00FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF;
    }
}
