// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";

import {AccessControlUpgradeable} from "@openzeppelin/contracts-upgradeable/access/AccessControlUpgradeable.sol";
import {ReentrancyGuardTransient} from "@openzeppelin/contracts/utils/ReentrancyGuardTransient.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {IzERC20} from "../interfaces/IzERC20.sol";
import {ILiquidityManager} from "../interfaces/ILiquidityManager.sol";
import {IncentiveLib} from "../libraries/IncentiveLib.sol";

/// @notice Custodies underlying token liquidity and mints/burns zERC20 based on wrap/unwrap flows.
/// Reward/fee curves follow the piecewise linear formulas described in docs/zerc20-liquidity.md.
/// @dev Liquidity is derived from the underlying token balance; direct transfers (donations) intentionally
///      affect incentive calculations and are not ignored by separate accounting.
contract LiquidityManager is UUPSUpgradeable, AccessControlUpgradeable, ReentrancyGuardTransient, ILiquidityManager {
    using SafeERC20 for IERC20;
    using IncentiveLib for IncentiveLib.FeeParams;

    /// @notice Role allowed to update incentive curve parameters.
    bytes32 public constant FEE_MANAGER_ROLE = keccak256("FEE_MANAGER");

    /// @notice ERC7528 native token address convention
    address private constant NATIVE_TOKEN = 0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE;

    IERC20 private immutable UNDERLYING_TOKEN;
    IzERC20 private immutable ZERC20_TOKEN;
    bool private immutable IS_NATIVE_UNDERLYING;

    error ZeroAddress();
    error ZeroAmount();
    error ZeroReceiver();
    error UnderlyingPullFailed();
    error UnderlyingSendFailed();
    error InsufficientLiquidity();
    error InsufficientRewards();
    error DecimalMismatch();
    error SlippageExceeded();
    error InvalidMsgValue(uint256 expected, uint256 actual);
    error NativeTokenNotSupported();

    // ERC-7201 slot for namespace "zerc20.storage.liquidityManager".
    bytes32 internal constant LIQUIDITY_MANAGER_STORAGE_SLOT =
        0x63c90750c40e4ec3ae62a755935b126c2e8aa4b2b6c7a4a02d9adec8efbbaa00;

    /// @custom:storage-location erc7201:zerc20.storage.liquidityManager
    struct LiquidityManagerStorage {
        IncentiveLib.FeeParams feeParams;
        uint256 feeSurplus; // Tracks collected fees net of distributed rewards.
    }

    /// @notice Emitted when fee parameters are updated.
    event FeeParamsUpdated(IncentiveLib.FeeParams params);
    /// @notice Emitted after a successful wrap.
    event Wrapped(address indexed caller, address indexed receiver, uint256 amountOut, uint256 reward);
    /// @notice Emitted after a successful unwrap.
    event Unwrapped(address indexed caller, address indexed receiver, uint256 amountOut, uint256 feeAmount);
    /// @notice Emitted after admin withdraws fee surplus.
    event RewardsWithdrawn(address indexed to, uint256 amount);

    /// @notice Locks implementation contracts on deployment.
    constructor(address underlyingToken_, address zerc20_) {
        require(underlyingToken_ != address(0) && zerc20_ != address(0), ZeroAddress());
        UNDERLYING_TOKEN = IERC20(underlyingToken_);
        ZERC20_TOKEN = IzERC20(zerc20_);
        IS_NATIVE_UNDERLYING = address(UNDERLYING_TOKEN) == NATIVE_TOKEN;
        _disableInitializers();
    }

    /// @notice Initializes the liquidity manager with fee params and admin roles.
    /// @param _feeParams Incentive curve parameters for rewards and fees.
    /// @param initialOwner Account receiving admin and fee-manager roles.
    function initialize(IncentiveLib.FeeParams calldata _feeParams, address initialOwner) external initializer {
        require(initialOwner != address(0), ZeroAddress());
        if (IS_NATIVE_UNDERLYING) {
            require(IERC20Metadata(address(ZERC20_TOKEN)).decimals() == 18, DecimalMismatch());
        } else {
            require(
                IERC20Metadata(address(ZERC20_TOKEN)).decimals()
                    == IERC20Metadata(address(UNDERLYING_TOKEN)).decimals(),
                DecimalMismatch()
            );
        }
        _feeParams.validateFeeParams();

        __AccessControl_init();
        _grantRole(DEFAULT_ADMIN_ROLE, initialOwner);
        _grantRole(FEE_MANAGER_ROLE, initialOwner);

        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        $.feeParams = _feeParams;
    }

    // ---------------------------- User ----------------------------------

    /// @notice Pulls underlying from the caller and mints zERC20 to `receiver`.
    /// @param amount Amount of underlying to deposit.
    /// @param receiver Address receiving minted zERC20.
    /// @return amountOut zERC20 minted, including any reward.
    function wrap(uint256 amount, address receiver) external payable nonReentrant returns (uint256 amountOut) {
        amountOut = _wrap(amount, receiver);
    }

    /// @notice Wraps with a minimum output check to enforce slippage constraints.
    /// @param amount Amount of underlying to deposit.
    /// @param minOut Minimum acceptable zERC20 minted.
    /// @param receiver Address receiving minted zERC20.
    /// @return amountOut zERC20 minted, including any reward.
    function wrapWithMinOut(uint256 amount, uint256 minOut, address receiver)
        external
        payable
        nonReentrant
        returns (uint256 amountOut)
    {
        amountOut = _wrap(amount, receiver);
        require(amountOut >= minOut, SlippageExceeded());
    }

    /// @notice Burns zERC20 from the caller and releases underlying to `receiver`.
    /// @param amount Amount of zERC20 to burn.
    /// @param receiver Address receiving the underlying.
    /// @return amountOut Underlying released after fees.
    function unwrap(uint256 amount, address receiver) external nonReentrant returns (uint256 amountOut) {
        amountOut = _unwrap(amount, receiver);
    }

    /// @notice Unwraps with a minimum output check to enforce slippage constraints.
    /// @param amount Amount of zERC20 to burn.
    /// @param minOut Minimum acceptable underlying released.
    /// @param receiver Address receiving the underlying.
    /// @return amountOut Underlying released after fees.
    function unwrapWithMinOut(uint256 amount, uint256 minOut, address receiver)
        external
        nonReentrant
        returns (uint256 amountOut)
    {
        amountOut = _unwrap(amount, receiver);
        require(amountOut >= minOut, SlippageExceeded());
    }

    // ---------------------------- Views ----------------------------------

    /// @notice Quotes reward paid for wrapping `amount` at current liquidity.
    /// @param amount Amount of underlying to wrap.
    /// @return reward Reward amount paid from fee surplus.
    function quoteWrapReward(uint256 amount) external view returns (uint256 reward) {
        return _quoteWrapReward(amount, _getLiquidityManagerStorage());
    }

    /// @notice Quotes fee charged for unwrapping `amount` at current liquidity.
    /// @param amount Amount of zERC20 to unwrap.
    /// @return feeAmount Fee charged in underlying units.
    function quoteUnwrapFee(uint256 amount) external view returns (uint256 feeAmount) {
        return _quoteUnwrapFee(amount, _getLiquidityManagerStorage());
    }

    /// @notice Returns the wrapped underlying token.
    function underlyingToken() external view returns (IERC20) {
        return UNDERLYING_TOKEN;
    }

    /// @notice Returns the zERC20 token minted/burned by this contract.
    function zerc20() external view returns (IzERC20) {
        return ZERC20_TOKEN;
    }

    /// @notice Returns the incentive curve parameters.
    function feeParams() external view returns (IncentiveLib.FeeParams memory params) {
        params = _getLiquidityManagerStorage().feeParams;
    }

    /// @notice Returns the fee surplus available for rewards and admin withdrawals.
    function feeSurplus() external view returns (uint256) {
        return _getLiquidityManagerStorage().feeSurplus;
    }

    // ---------------------------- Admin ------------------------------------

    /// @notice Updates the incentive curve parameters.
    /// @param params New fee parameters.
    function setFeeParams(IncentiveLib.FeeParams calldata params) external onlyRole(FEE_MANAGER_ROLE) {
        params.validateFeeParams();
        _getLiquidityManagerStorage().feeParams = params;
        emit FeeParamsUpdated(params);
    }

    /// @notice Withdraws accumulated fees to a specified address
    /// @dev Only callable by admin. Uses low-level call for native token transfers.
    /// @param to Recipient address (validated non-zero)
    /// @param amount Amount to withdraw (validated <= feeSurplus)
    function withdrawRewards(address to, uint256 amount) external nonReentrant onlyRole(DEFAULT_ADMIN_ROLE) {
        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        require(to != address(0), ZeroReceiver());
        require(amount != 0, ZeroAmount());
        require(amount <= $.feeSurplus, InsufficientRewards());

        $.feeSurplus -= amount;
        if (IS_NATIVE_UNDERLYING) {
            // slither-disable-next-line arbitrary-send-eth
            (bool success,) = payable(to).call{value: amount}("");
            require(success, UnderlyingSendFailed());
        } else {
            UNDERLYING_TOKEN.safeTransfer(to, amount);
        }
        emit RewardsWithdrawn(to, amount);
    }

    // ---------------------------- Internal ----------------------------------

    /// @dev Returns the storage pointer for ERC-7201 layout.
    function _getLiquidityManagerStorage() private pure returns (LiquidityManagerStorage storage $) {
        bytes32 slot = LIQUIDITY_MANAGER_STORAGE_SLOT;
        // solhint-disable-next-line no-inline-assembly
        assembly {
            $.slot := slot
        }
    }

    function _underlyingBalance() private view returns (uint256) {
        if (IS_NATIVE_UNDERLYING) {
            return address(this).balance;
        }
        return UNDERLYING_TOKEN.balanceOf(address(this));
    }

    /// @dev Quotes wrapping reward using current token balance and fee surplus.
    function _quoteWrapReward(uint256 amount, LiquidityManagerStorage storage $) private view returns (uint256 reward) {
        uint256 balance = _underlyingBalance();
        uint256 feeSurplus_ = $.feeSurplus;
        // @note: underflow is unlikely here but possible if balance of underlying token changes externally.
        uint256 liquidity = balance >= feeSurplus_ ? balance - feeSurplus_ : 0;
        reward = $.feeParams.quoteWrapReward(liquidity, feeSurplus_, amount);
    }

    /// @dev Quotes unwrap fee using current token balance and fee surplus.
    function _quoteUnwrapFee(uint256 amount, LiquidityManagerStorage storage $)
        private
        view
        returns (uint256 feeAmount)
    {
        uint256 balance = _underlyingBalance();
        uint256 feeSurplus_ = $.feeSurplus;
        // @note: underflow is unlikely here but possible if balance of underlying token changes externally.
        uint256 liquidity = balance >= feeSurplus_ ? balance - feeSurplus_ : 0;
        feeAmount = $.feeParams.quoteUnwrapFee(liquidity, amount);
    }

    /// @dev Internal wrap implementation using pre-deposit liquidity for reward quotes.
    function _wrap(uint256 amount, address receiver) private returns (uint256 amountOut) {
        require(amount != 0, ZeroAmount());
        require(receiver != address(0), ZeroReceiver());

        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        /// @dev for native, msg.value is already in address(this).balance; for ERC20, balance updates after transferFrom.
        uint256 balanceBefore = IS_NATIVE_UNDERLYING ? address(this).balance - msg.value : _underlyingBalance();
        uint256 received;

        if (IS_NATIVE_UNDERLYING) {
            require(msg.value == amount, InvalidMsgValue(amount, msg.value));
            received = msg.value;
        } else {
            require(msg.value == 0, InvalidMsgValue(0, msg.value));
            UNDERLYING_TOKEN.safeTransferFrom(msg.sender, address(this), amount);
            received = _underlyingBalance() - balanceBefore;
        }
        require(received != 0, UnderlyingPullFailed());

        uint256 feeSurplus_ = $.feeSurplus;
        // Keep reward calculation aligned with pre-deposit liquidity.
        uint256 liquidityBefore = balanceBefore >= feeSurplus_ ? balanceBefore - feeSurplus_ : 0;
        uint256 reward = $.feeParams.quoteWrapReward(liquidityBefore, feeSurplus_, received);

        if (reward > 0) $.feeSurplus -= reward;
        amountOut = received + reward;
        ZERC20_TOKEN.mint(receiver, amountOut);
        emit Wrapped(msg.sender, receiver, amountOut, reward);
    }

    /// @dev Internal unwrap implementation that burns zERC20 and transfers underlying.
    function _unwrap(uint256 amount, address receiver) private returns (uint256 amountOut) {
        require(amount != 0, ZeroAmount());
        require(receiver != address(0), ZeroReceiver());

        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        uint256 feeAmount = _quoteUnwrapFee(amount, $);
        amountOut = amount - feeAmount;

        // slither-disable-next-line reentrancy-balance
        ZERC20_TOKEN.burn(msg.sender, amount);
        if (amountOut > 0) {
            if (IS_NATIVE_UNDERLYING) {
                require(address(this).balance >= amountOut, InsufficientLiquidity());
                // slither-disable-next-line arbitrary-send-eth
                (bool success,) = payable(receiver).call{value: amountOut}("");
                require(success, UnderlyingSendFailed());
            } else {
                require(UNDERLYING_TOKEN.balanceOf(address(this)) >= amountOut, InsufficientLiquidity());
                UNDERLYING_TOKEN.safeTransfer(receiver, amountOut);
            }
        }
        if (feeAmount > 0) $.feeSurplus += feeAmount;

        emit Unwrapped(msg.sender, receiver, amountOut, feeAmount);
    }

    /// @dev Restricts upgrade authorization to admins.
    function _authorizeUpgrade(address) internal override onlyRole(DEFAULT_ADMIN_ROLE) {}

    // solhint-disable-next-line no-complex-fallback
    receive() external payable nonReentrant {
        require(IS_NATIVE_UNDERLYING, NativeTokenNotSupported());
        _wrap(msg.value, msg.sender);
    }
}
