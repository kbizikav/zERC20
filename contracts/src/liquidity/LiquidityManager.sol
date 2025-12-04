// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {AccessControlUpgradeable} from "@openzeppelin/contracts-upgradeable/access/AccessControlUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {SlotDerivation} from "@openzeppelin/contracts/utils/SlotDerivation.sol";
import {IzERC20} from "../interfaces/IzERC20.sol";
import {ILiquidityManager} from "../interfaces/ILiquidityManager.sol";
import {FeeLib} from "../libraries/FeeLib.sol";

/// @notice Custodies underlying token liquidity and mints/burns zERC20 based on wrap/unwrap flows.
/// Reward/fee curves follow the piecewise linear formulas described in docs/zerc20-liquidity.md.
contract LiquidityManager is Initializable, UUPSUpgradeable, AccessControlUpgradeable, ILiquidityManager {
    using SlotDerivation for string;

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

    /// @custom:storage-location erc7201:zerc20.storage.liquidityManager
    struct LiquidityManagerStorage {
        IERC20 underlyingToken;
        IzERC20 zerc20;
        uint256 lTarget;
        FeeLib.RewardParams rewardParams;
        FeeLib.FeeParams feeParams;
        uint256 feeSurplus; // Tracks collected fees net of distributed rewards.
    }

    event TargetUpdated(uint256 lTarget);
    event RewardParamsUpdated(FeeLib.RewardParams params);
    event FeeParamsUpdated(FeeLib.FeeParams params);
    event Wrapped(address indexed caller, address indexed receiver, uint256 amountOut, uint256 reward);
    event Unwrapped(address indexed caller, address indexed receiver, uint256 amountOut, uint256 feeAmount);
    event RewardsWithdrawn(address indexed to, uint256 amount);

    constructor() {
        _disableInitializers();
    }

    function _getLiquidityManagerStorage() private pure returns (LiquidityManagerStorage storage $) {
        bytes32 slot = SlotDerivation.erc7201Slot("zerc20.storage.liquidityManager");
        assembly {
            $.slot := slot
        }
    }

    function initialize(
        address _underlyingToken,
        address _zerc20,
        uint256 _lTarget,
        FeeLib.RewardParams memory _rewardParams,
        FeeLib.FeeParams memory _feeParams,
        address initialOwner
    ) external initializer {
        if (_underlyingToken == address(0) || _zerc20 == address(0)) revert ZeroAddress();
        if (_lTarget == 0) revert InvalidTarget();
        _validateRewardParams(_rewardParams);
        _validateFeeParams(_feeParams);

        __AccessControl_init();
        __UUPSUpgradeable_init();
        _grantRole(DEFAULT_ADMIN_ROLE, initialOwner);
        _grantRole(FEE_MANAGER_ROLE, initialOwner);

        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        $.underlyingToken = IERC20(_underlyingToken);
        $.zerc20 = IzERC20(_zerc20);
        $.lTarget = _lTarget;
        $.rewardParams = _rewardParams;
        $.feeParams = _feeParams;
    }

    function underlyingToken() public view returns (IERC20) {
        return _getLiquidityManagerStorage().underlyingToken;
    }

    function zerc20() public view returns (IzERC20) {
        return _getLiquidityManagerStorage().zerc20;
    }

    function lTarget() public view returns (uint256) {
        return _getLiquidityManagerStorage().lTarget;
    }

    function rewardParams() public view returns (FeeLib.RewardParams memory params) {
        params = _getLiquidityManagerStorage().rewardParams;
    }

    function feeParams() public view returns (FeeLib.FeeParams memory params) {
        params = _getLiquidityManagerStorage().feeParams;
    }

    function feeSurplus() public view returns (uint256) {
        return _getLiquidityManagerStorage().feeSurplus;
    }

    // ---------------------------- User ----------------------------------

    function wrap(uint256 amount, address receiver) external returns (uint256 amountOut) {
        if (amount == 0) revert ZeroAmount();
        if (receiver == address(0)) revert ZeroReceiver();

        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        uint256 reward = _quoteWrap(amount, $);

        if (!$.underlyingToken.transferFrom(msg.sender, address(this), amount)) revert UnderlyingPullFailed();

        if (reward > 0) {
            $.feeSurplus -= reward;
        }
        amountOut = amount + reward;
        $.zerc20.mint(receiver, amountOut);
        emit Wrapped(msg.sender, receiver, amountOut, reward);
    }

    function unwrap(uint256 amount, address receiver) external returns (uint256 amountOut) {
        if (amount == 0) revert ZeroAmount();
        if (receiver == address(0)) revert ZeroReceiver();

        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        uint256 feeAmount = _quoteUnwrap(amount, $);
        amountOut = amount - feeAmount;

        $.zerc20.burn(msg.sender, amount);
        IERC20 underlying = $.underlyingToken;
        if (underlying.balanceOf(address(this)) < amountOut) revert InsufficientLiquidity();
        if (!underlying.transfer(receiver, amountOut)) revert UnderlyingSendFailed();
        $.feeSurplus += feeAmount;

        emit Unwrapped(msg.sender, receiver, amountOut, feeAmount);
    }

    function quoteWrap(uint256 amount) public view returns (uint256 reward) {
        return _quoteWrap(amount, _getLiquidityManagerStorage());
    }

    function quoteUnwrap(uint256 amount) public view returns (uint256 feeAmount) {
        return _quoteUnwrap(amount, _getLiquidityManagerStorage());
    }

    // ---------------------------- Admin ------------------------------------

    function setTarget(uint256 _lTarget) external onlyRole(FEE_MANAGER_ROLE) {
        if (_lTarget == 0) revert InvalidTarget();
        _getLiquidityManagerStorage().lTarget = _lTarget;
        emit TargetUpdated(_lTarget);
    }

    function setRewardParams(FeeLib.RewardParams calldata params) external onlyRole(FEE_MANAGER_ROLE) {
        _validateRewardParams(params);
        _getLiquidityManagerStorage().rewardParams = params;
        emit RewardParamsUpdated(params);
    }

    function setFeeParams(FeeLib.FeeParams calldata params) external onlyRole(FEE_MANAGER_ROLE) {
        _validateFeeParams(params);
        _getLiquidityManagerStorage().feeParams = params;
        emit FeeParamsUpdated(params);
    }

    function withdrawRewards(address to, uint256 amount) external onlyRole(DEFAULT_ADMIN_ROLE) {
        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        if (to == address(0)) revert ZeroReceiver();
        if (amount == 0) revert ZeroAmount();
        if (amount > $.feeSurplus) revert InsufficientRewards();

        $.feeSurplus -= amount;
        if (!$.underlyingToken.transfer(to, amount)) revert UnderlyingSendFailed();
        emit RewardsWithdrawn(to, amount);
    }

    // ---------------------------- Internal ----------------------------------

    function _quoteWrap(uint256 amount, LiquidityManagerStorage storage $)
        internal
        view
        returns (uint256 reward)
    {
        uint256 liquidityBefore = $.underlyingToken.balanceOf(address(this));
        uint256 rewardAmount = FeeLib.quoteWrap(amount, liquidityBefore, $.lTarget, $.rewardParams);
        uint256 surplus = $.feeSurplus;

        if (rewardAmount > 0 && surplus > 0) {
            reward = rewardAmount > surplus ? surplus : rewardAmount;
        }
    }

    function _quoteUnwrap(uint256 amount, LiquidityManagerStorage storage $)
        internal
        view
        returns (uint256 feeAmount)
    {
        uint256 liquidityBefore = $.underlyingToken.balanceOf(address(this));
        feeAmount = FeeLib.quoteUnwrap(amount, liquidityBefore, $.lTarget, $.feeParams);
    }

    function _validateRewardParams(FeeLib.RewardParams memory params) internal pure {
        params; // no-op; kept for symmetry and future validation hooks.
    }

    function _validateFeeParams(FeeLib.FeeParams memory params) internal pure {
        if (params.delta1Bps > BPS || params.delta2Bps > BPS) revert InvalidDeltas();
        if (params.delta1Bps <= params.delta2Bps || params.delta2Bps == 0) revert InvalidDeltaOrder();
        if (params.lambda1Bps > BPS) revert InvalidLambda1();
        if (params.lambda2Bps > BPS) revert InvalidLambda2();
    }

    function _authorizeUpgrade(address) internal override onlyRole(DEFAULT_ADMIN_ROLE) {}
}
