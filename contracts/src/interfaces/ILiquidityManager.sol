// SPDX-License-Identifier: BUSL-1.1
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IzERC20} from "./IzERC20.sol";

/// @dev External-facing interface for LiquidityManager so adaptors and callers can interact cleanly.
interface ILiquidityManager {
    function wrap(uint256 amount, address receiver) external payable returns (uint256 amountOut);

    function wrapWithMinOut(uint256 amount, uint256 minOut, address receiver)
        external
        payable
        returns (uint256 amountOut);

    function unwrap(uint256 amount, address receiver) external returns (uint256 amountOut);

    function unwrapWithMinOut(uint256 amount, uint256 minOut, address receiver) external returns (uint256 amountOut);

    function quoteWrapReward(uint256 amount) external view returns (uint256 rewardAmount);

    function quoteUnwrapFee(uint256 amount) external view returns (uint256 feeAmount);

    function underlyingToken() external view returns (IERC20);

    function zerc20() external view returns (IzERC20);

    function feeSurplus() external view returns (uint256);

    function withdrawRewards(address to, uint256 amount) external;
}
