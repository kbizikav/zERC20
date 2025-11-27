// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {AccessControlUpgradeable} from "@openzeppelin/contracts-upgradeable/access/AccessControlUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {IzERC20} from "./interfaces/IzERC20.sol";
import {ILiquidityManager} from "./interfaces/ILiquidityManager.sol";
import {FeeLib} from "./libraries/FeeLib.sol";

/// @notice Custodies underlying token liquidity and mints/burns zERC20 based on wrap/unwrap flows.
/// Reward/fee curves follow the piecewise linear formulas described in docs/zerc20-liquidity.md.
contract LiquidityManager is Initializable, UUPSUpgradeable, AccessControlUpgradeable, ILiquidityManager {
    uint256 private constant BPS = 10_000;
    bytes32 public constant FEE_MANAGER_ROLE = keccak256("FEE_MANAGER");

    error ZeroAddress();
    error InvalidTarget();
    error ZeroAmount();
    error ZeroReceiver();
    error UnderlyingPullFailed();
    error UnderlyingSendFailed();
    error InsufficientLiquidity();
    error InsufficientRewards();
    error InvalidDeltas();
    error InvalidDeltaOrder();
    error InvalidLambda1();
    error InvalidLambda2();

    IERC20 public underlyingToken;
    IzERC20 public zerc20;

    uint256 public lTarget;
    FeeLib.RewardParams public rewardParams;
    FeeLib.FeeParams public feeParams;
    uint256 public feeSurplus; // Tracks collected fees net of distributed rewards.

    event TargetUpdated(uint256 lTarget);
    event RewardParamsUpdated(FeeLib.RewardParams params);
    event FeeParamsUpdated(FeeLib.FeeParams params);
    event Wrapped(address indexed caller, address indexed receiver, uint256 amountOut, uint256 reward);
    event Unwrapped(address indexed caller, address indexed receiver, uint256 amountOut, uint256 feeAmount);
    event RewardsWithdrawn(address indexed to, uint256 amount);

    constructor() {
        _disableInitializers();
    }

    function initialize(
        address _underlyingToken,
        address _zerc20,
        uint256 _lTarget,
        FeeLib.RewardParams memory _rewardParams,
        FeeLib.FeeParams memory _feeParams,
        address initialOwner
    ) external initializer {
        require(_underlyingToken != address(0) && _zerc20 != address(0), ZeroAddress());
        require(_lTarget > 0, InvalidTarget());
        _validateRewardParams(_rewardParams);
        _validateFeeParams(_feeParams);

        __AccessControl_init();
        _grantRole(DEFAULT_ADMIN_ROLE, initialOwner);
        _grantRole(FEE_MANAGER_ROLE, initialOwner);

        underlyingToken = IERC20(_underlyingToken);
        zerc20 = IzERC20(_zerc20);
        lTarget = _lTarget;
        rewardParams = _rewardParams;
        feeParams = _feeParams;
    }

    // ---------------------------- User ----------------------------------

    function wrap(uint256 amount, address receiver) external returns (uint256 amountOut) {
        require(amount > 0, ZeroAmount());
        require(receiver != address(0), ZeroReceiver());

        uint256 reward = quoteWrap(amount);

        require(underlyingToken.transferFrom(msg.sender, address(this), amount), UnderlyingPullFailed());

        if (reward > 0) {
            feeSurplus -= reward;
        }
        amountOut = amount + reward;
        zerc20.mint(receiver, amountOut);
        emit Wrapped(msg.sender, receiver, amountOut, reward);
    }

    function unwrap(uint256 amount, address receiver) external returns (uint256 amountOut) {
        require(amount > 0, ZeroAmount());
        require(receiver != address(0), ZeroReceiver());

        uint256 feeAmount = quoteUnwrap(amount);
        amountOut = amount - feeAmount;

        zerc20.burn(msg.sender, amount);
        require(underlyingToken.balanceOf(address(this)) >= amountOut, InsufficientLiquidity());
        require(underlyingToken.transfer(receiver, amountOut), UnderlyingSendFailed());
        feeSurplus += feeAmount;

        emit Unwrapped(msg.sender, receiver, amountOut, feeAmount);
    }

    function quoteWrap(uint256 amount) public view returns (uint256 reward) {
        uint256 liquidityBefore = underlyingToken.balanceOf(address(this));
        uint256 rewardAmount = FeeLib.quoteWrap(amount, liquidityBefore, lTarget, rewardParams);

        if (rewardAmount > 0 && feeSurplus > 0) {
            reward = rewardAmount > feeSurplus ? feeSurplus : rewardAmount;
        }
    }

    function quoteUnwrap(uint256 amount) public view returns (uint256 feeAmount) {
        uint256 liquidityBefore = underlyingToken.balanceOf(address(this));
        return FeeLib.quoteUnwrap(amount, liquidityBefore, lTarget, feeParams);
    }

    // ---------------------------- Admin ------------------------------------

    function setTarget(uint256 _lTarget) external onlyRole(FEE_MANAGER_ROLE) {
        require(_lTarget > 0, InvalidTarget());
        lTarget = _lTarget;
        emit TargetUpdated(_lTarget);
    }

    function setRewardParams(FeeLib.RewardParams calldata params) external onlyRole(FEE_MANAGER_ROLE) {
        _validateRewardParams(params);
        rewardParams = params;
        emit RewardParamsUpdated(params);
    }

    function setFeeParams(FeeLib.FeeParams calldata params) external onlyRole(FEE_MANAGER_ROLE) {
        _validateFeeParams(params);
        feeParams = params;
        emit FeeParamsUpdated(params);
    }

    function withdrawRewards(address to, uint256 amount) external onlyRole(DEFAULT_ADMIN_ROLE) {
        require(to != address(0), ZeroReceiver());
        require(amount > 0, ZeroAmount());
        require(amount <= feeSurplus, InsufficientRewards());

        feeSurplus -= amount;
        require(underlyingToken.transfer(to, amount), UnderlyingSendFailed());
        emit RewardsWithdrawn(to, amount);
    }

    // ---------------------------- Internal ----------------------------------

    function _validateRewardParams(FeeLib.RewardParams memory params) internal pure {
        params; // no-op; kept for symmetry and future validation hooks.
    }

    function _validateFeeParams(FeeLib.FeeParams memory params) internal pure {
        require(params.delta1Bps <= BPS && params.delta2Bps <= BPS, InvalidDeltas());
        require(params.delta1Bps > params.delta2Bps && params.delta2Bps > 0, InvalidDeltaOrder());
        require(params.lambda1Bps <= BPS, InvalidLambda1());
        require(params.lambda2Bps <= BPS, InvalidLambda2());
    }

    function _authorizeUpgrade(address) internal override onlyRole(DEFAULT_ADMIN_ROLE) {}

    uint256[44] private __gap;
}
