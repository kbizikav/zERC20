// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IERC20Upgradeable} from "@openzeppelin/contracts-upgradeable/token/ERC20/IERC20Upgradeable.sol";

/// @title IzERC20
/// @notice zERC20 interface that extends ERC20 with teleport semantics and indexed transfer hashing.
interface IzERC20 is IERC20Upgradeable {
    /// @notice Emitted after every transfer, capturing the leaf index and transfer tuple.
    event IndexedTransfer(uint256 indexed index, address from, address to, uint256 value);

    /// @notice Emitted when a verifier-authorized teleport mints new tokens.
    event Teleport(address indexed to, uint256 value);

    /// @notice Hash chain committing every transfer's destination and amount pair.
    /// @return chain Hash chain accumulator.
    function hashChain() external view returns (uint256 chain);

    /// @notice Index of the next transfer, aligned with the off-chain Merkle tree leaf position.
    /// @return nextIndex Transfer index counter.
    function index() external view returns (uint256 nextIndex);

    /// @notice Mints tokens according to a proof validated by the verifier.
    /// @param to Recipient address that receives the minted tokens.
    /// @param value Amount of tokens to mint.
    function teleport(address to, uint256 value) external;
}
