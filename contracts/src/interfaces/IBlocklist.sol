// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title IBlocklist
/// @notice Interface for the shared blocklist registry.
interface IBlocklist {
    /// @notice Returns whether an address is blocked.
    function isBlocked(address account) external view returns (bool);
}
