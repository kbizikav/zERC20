// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {AccessControlUpgradeable} from "@openzeppelin/contracts-upgradeable/access/AccessControlUpgradeable.sol";
import {ReentrancyGuardUpgradeable} from "@openzeppelin/contracts-upgradeable/security/ReentrancyGuardUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {SlotDerivation} from "@openzeppelin/contracts/utils/SlotDerivation.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {IzERC20} from "../interfaces/IzERC20.sol";
import {ILiquidityManager} from "../interfaces/ILiquidityManager.sol";
import {IncentiveLib} from "../libraries/IncentiveLib.sol";

/// @notice Custodies underlying token liquidity and mints/burns zERC20 based on wrap/unwrap flows.
/// Reward/fee curves follow the piecewise linear formulas described in docs/zerc20-liquidity.md.
/// @dev Liquidity is derived from the underlying token balance; direct transfers (donations) intentionally
///      affect incentive calculations and are not ignored by separate accounting.
contract LiquidityManager is
    Initializable,
    UUPSUpgradeable,
    AccessControlUpgradeable,
    ReentrancyGuardUpgradeable,
    ILiquidityManager
{
    using SafeERC20 for IERC20;
    using SlotDerivation for string;

    bytes32 public constant FEE_MANAGER_ROLE = keccak256("FEE_MANAGER");

    error ZeroAddress();
    error ZeroAmount();
    error ZeroReceiver();
    error UnderlyingPullFailed();
    error UnderlyingSendFailed();
    error InsufficientLiquidity();
    error InsufficientRewards();
    error DecimalMismatch();
    error SlippageExceeded();

    /// @custom:storage-location erc7201:zerc20.storage.liquidityManager
    struct LiquidityManagerStorage {
        IERC20 underlyingToken;
        IzERC20 zerc20;
        IncentiveLib.FeeParams feeParams;
        uint256 feeSurplus; // Tracks collected fees net of distributed rewards.
    }

    event FeeParamsUpdated(IncentiveLib.FeeParams params);
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
        IncentiveLib.FeeParams memory _feeParams,
        address initialOwner
    ) external initializer {
        if (_underlyingToken == address(0) || _zerc20 == address(0) || initialOwner == address(0)) {
            revert ZeroAddress();
        }
        if (IERC20Metadata(_zerc20).decimals() != IERC20Metadata(_underlyingToken).decimals()) revert DecimalMismatch();
        IncentiveLib._validateFeeParams(_feeParams);

        __AccessControl_init();
        __ReentrancyGuard_init();
        __UUPSUpgradeable_init();
        _grantRole(DEFAULT_ADMIN_ROLE, initialOwner);
        _grantRole(FEE_MANAGER_ROLE, initialOwner);

        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        $.underlyingToken = IERC20(_underlyingToken);
        $.zerc20 = IzERC20(_zerc20);
        $.feeParams = _feeParams;
    }

    function underlyingToken() public view returns (IERC20) {
        return _getLiquidityManagerStorage().underlyingToken;
    }

    function zerc20() public view returns (IzERC20) {
        return _getLiquidityManagerStorage().zerc20;
    }

    function feeParams() public view returns (IncentiveLib.FeeParams memory params) {
        params = _getLiquidityManagerStorage().feeParams;
    }

    function feeSurplus() public view returns (uint256) {
        return _getLiquidityManagerStorage().feeSurplus;
    }

    // ---------------------------- User ----------------------------------

    function wrap(uint256 amount, address receiver) external nonReentrant returns (uint256 amountOut) {
        amountOut = _wrap(amount, receiver);
    }

    function wrapWithMinOut(uint256 amount, uint256 minOut, address receiver)
        external
        nonReentrant
        returns (uint256 amountOut)
    {
        amountOut = _wrap(amount, receiver);
        if (amountOut < minOut) revert SlippageExceeded();
    }

    function unwrap(uint256 amount, address receiver) external nonReentrant returns (uint256 amountOut) {
        amountOut = _unwrap(amount, receiver);
    }

    function unwrapWithMinOut(uint256 amount, uint256 minOut, address receiver)
        external
        nonReentrant
        returns (uint256 amountOut)
    {
        amountOut = _unwrap(amount, receiver);
        if (amountOut < minOut) revert SlippageExceeded();
    }

    function quoteWrapReward(uint256 amount) public view returns (uint256 reward) {
        return _quoteWrapReward(amount, _getLiquidityManagerStorage());
    }

    function quoteUnwrapFee(uint256 amount) public view returns (uint256 feeAmount) {
        return _quoteUnwrapFee(amount, _getLiquidityManagerStorage());
    }

    // ---------------------------- Admin ------------------------------------

    function setFeeParams(IncentiveLib.FeeParams calldata params) external onlyRole(FEE_MANAGER_ROLE) {
        IncentiveLib._validateFeeParams(params);
        _getLiquidityManagerStorage().feeParams = params;
        emit FeeParamsUpdated(params);
    }

    function withdrawRewards(address to, uint256 amount) external nonReentrant onlyRole(DEFAULT_ADMIN_ROLE) {
        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        if (to == address(0)) revert ZeroReceiver();
        if (amount == 0) revert ZeroAmount();
        if (amount > $.feeSurplus) revert InsufficientRewards();

        $.feeSurplus -= amount;
        $.underlyingToken.safeTransfer(to, amount);
        emit RewardsWithdrawn(to, amount);
    }

    // ---------------------------- Internal ----------------------------------

    function _quoteWrapReward(uint256 amount, LiquidityManagerStorage storage $)
        internal
        view
        returns (uint256 reward)
    {
        uint256 balance = $.underlyingToken.balanceOf(address(this));
        uint256 feeSurplus_ = $.feeSurplus;
        // @note: underflow is unlikely here but possible if balance of underlying token changes externally.
        uint256 liquidity = balance >= feeSurplus_ ? balance - feeSurplus_ : 0;
        reward = IncentiveLib.quoteWrapReward($.feeParams, liquidity, feeSurplus_, amount);
    }

    function _quoteUnwrapFee(uint256 amount, LiquidityManagerStorage storage $)
        internal
        view
        returns (uint256 feeAmount)
    {
        uint256 balance = $.underlyingToken.balanceOf(address(this));
        uint256 feeSurplus_ = $.feeSurplus;
        // @note: underflow is unlikely here but possible if balance of underlying token changes externally.
        uint256 liquidity = balance >= feeSurplus_ ? balance - feeSurplus_ : 0;
        feeAmount = IncentiveLib.quoteUnwrapFee($.feeParams, liquidity, amount);
    }

    function _wrap(uint256 amount, address receiver) internal returns (uint256 amountOut) {
        if (amount == 0) revert ZeroAmount();
        if (receiver == address(0)) revert ZeroReceiver();

        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        IERC20 underlying = $.underlyingToken;
        uint256 balanceBefore = underlying.balanceOf(address(this));

        underlying.safeTransferFrom(msg.sender, address(this), amount);
        uint256 received = underlying.balanceOf(address(this)) - balanceBefore;
        if (received == 0) revert UnderlyingPullFailed();

        uint256 feeSurplus_ = $.feeSurplus;
        // Keep reward calculation aligned with pre-deposit liquidity.
        uint256 liquidityBefore = balanceBefore >= feeSurplus_ ? balanceBefore - feeSurplus_ : 0;
        uint256 reward = IncentiveLib.quoteWrapReward($.feeParams, liquidityBefore, feeSurplus_, received);

        if (reward > 0) $.feeSurplus -= reward;
        amountOut = received + reward;
        $.zerc20.mint(receiver, amountOut);
        emit Wrapped(msg.sender, receiver, amountOut, reward);
    }

    function _unwrap(uint256 amount, address receiver) internal returns (uint256 amountOut) {
        if (amount == 0) revert ZeroAmount();
        if (receiver == address(0)) revert ZeroReceiver();

        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        uint256 feeAmount = _quoteUnwrapFee(amount, $);
        amountOut = amount - feeAmount;

        $.zerc20.burn(msg.sender, amount);
        IERC20 underlying = $.underlyingToken;
        if (amountOut > 0) {
            if (underlying.balanceOf(address(this)) < amountOut) revert InsufficientLiquidity();
            underlying.safeTransfer(receiver, amountOut);
        }
        if (feeAmount > 0) $.feeSurplus += feeAmount;

        emit Unwrapped(msg.sender, receiver, amountOut, feeAmount);
    }

    function _authorizeUpgrade(address) internal override onlyRole(DEFAULT_ADMIN_ROLE) {}
}
