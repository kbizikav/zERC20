// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title ShaHashChainLib
/// @notice Library for computing hash chains used in ZK transfer verification.
library ShaHashChainLib {
    /// @dev Mask to clear upper 8 bits, keeping lower 248 bits for field element compatibility.
    uint256 private constant LOWER_248_BITS_MASK = type(uint256).max >> 8;

    /// @notice Computes the next hash in the transfer chain.
    /// @dev Computes sha256(BE(prev,32) || BE(from,20) || BE(to,20) || BE(value,32))
    ///      and returns the lower 248 bits (most-significant byte dropped).
    /// @param prev The previous hash in the chain (or initial value).
    /// @param from The sender address.
    /// @param to The recipient address.
    /// @param value The transfer amount.
    /// @return next The computed hash with upper 8 bits cleared.
    function compute(uint256 prev, address from, address to, uint256 value) internal pure returns (uint256 next) {
        next = uint256(sha256(abi.encodePacked(prev, from, to, value))) & LOWER_248_BITS_MASK;
    }
}
