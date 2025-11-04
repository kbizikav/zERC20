// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/**
 * @title IMintableBurnableERC20
 * @notice Minimal interface for ERC20 tokens that expose mint and burn entrypoints.
 */
interface IMintableBurnableERC20 {
    function mint(address to, uint256 amount) external;

    function burn(address from, uint256 amount) external;
}

