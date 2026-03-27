// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

/// @title Blocklist
/// @notice Shared OFAC-sanctioned-address registry referenced by multiple zERC20 tokens on the same chain.
contract Blocklist is Ownable {
    mapping(address => bool) private _blocked;

    /// @notice Emitted when an address is added to the blocklist.
    event AddressBlocked(address indexed account);
    /// @notice Emitted when an address is removed from the blocklist.
    event AddressUnblocked(address indexed account);

    /// @notice Reverts when the zero address is passed.
    error ZeroAddress();

    constructor(address initialOwner) Ownable(initialOwner) {}

    /// @notice Returns whether an address is blocked.
    /// @param account Address to query.
    /// @return True if the address is on the blocklist.
    function isBlocked(address account) external view returns (bool) {
        return _blocked[account];
    }

    /// @notice Adds an address to the blocklist.
    /// @param account Address to block.
    function blockAddress(address account) external onlyOwner {
        require(account != address(0), ZeroAddress());
        _blocked[account] = true;
        emit AddressBlocked(account);
    }

    /// @notice Removes an address from the blocklist.
    /// @param account Address to unblock.
    function unblockAddress(address account) external onlyOwner {
        require(account != address(0), ZeroAddress());
        _blocked[account] = false;
        emit AddressUnblocked(account);
    }

    /// @notice Adds multiple addresses to the blocklist in one transaction.
    /// @param accounts Addresses to block.
    function blockAddresses(address[] calldata accounts) external onlyOwner {
        for (uint256 i; i < accounts.length; ++i) {
            require(accounts[i] != address(0), ZeroAddress());
            _blocked[accounts[i]] = true;
            emit AddressBlocked(accounts[i]);
        }
    }
}
