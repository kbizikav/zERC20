// SPDX-License-Identifier: MIT
pragma solidity 0.8.30; // comment リリース前なのであれば、0.8.33を使ってもいいと思います。
// comment このソースに限らず
//   forge fmt
//   forge lint
//   slither
//   solhint
// でチェックして、そこを直すとより品質良くなると思います。

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
    Initializable, // comment AccessControlUpgradeableがInitializableを継承しているので、ここで継承する意味はないかと思います。
    UUPSUpgradeable,
    AccessControlUpgradeable,
    ReentrancyGuardUpgradeable,
    ILiquidityManager
{
    using SafeERC20 for IERC20;
    using SlotDerivation for string;

    /// @notice Role allowed to update incentive curve parameters.
    bytes32 public constant FEE_MANAGER_ROLE = keccak256("FEE_MANAGER");

    /// @notice ERC7528 native token address convention
    address internal constant NATIVE_TOKEN = 0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE;

    // comment errorはinterfaceに移動したほうがいいですね。
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

    // comment structはinterfaceに移動したほうがいいですね。
    /// @custom:storage-location erc7201:zerc20.storage.liquidityManager
    struct LiquidityManagerStorage {
        IERC20 underlyingToken;
        IzERC20 zerc20;
        IncentiveLib.FeeParams feeParams;
        uint256 feeSurplus; // Tracks collected fees net of distributed rewards.
    }

    // comment eventはinterfaceに移動したほうがいいですね。
    /// @notice Emitted when fee parameters are updated.
    event FeeParamsUpdated(IncentiveLib.FeeParams params);
    /// @notice Emitted after a successful wrap.
    event Wrapped(address indexed caller, address indexed receiver, uint256 amountOut, uint256 reward);
    /// @notice Emitted after a successful unwrap.
    event Unwrapped(address indexed caller, address indexed receiver, uint256 amountOut, uint256 feeAmount);
    /// @notice Emitted after admin withdraws fee surplus.
    event RewardsWithdrawn(address indexed to, uint256 amount);

    /// @notice Locks implementation contracts on deployment.
    constructor() {
        _disableInitializers();
    }

    /// @notice Initializes the liquidity manager with tokens, fee params, and admin roles.
    /// @param _underlyingToken ERC20 token being wrapped.
    /// @param _zerc20 zERC20 token minted/burned by this contract.
    /// @param _feeParams Incentive curve parameters for rewards and fees.
    /// @param initialOwner Account receiving admin and fee-manager roles.
    function initialize(
        address _underlyingToken,
        address _zerc20,
        IncentiveLib.FeeParams memory _feeParams,
        address initialOwner
    ) external initializer {
        if (_underlyingToken == address(0) || _zerc20 == address(0) || initialOwner == address(0)) {
            revert ZeroAddress();
        }
        bool isNative = _underlyingToken == NATIVE_TOKEN;
        if (isNative) {
            if (IERC20Metadata(_zerc20).decimals() != 18) revert DecimalMismatch();
        } else {
            if (IERC20Metadata(_zerc20).decimals() != IERC20Metadata(_underlyingToken).decimals()) {
                revert DecimalMismatch();
            }
        }
        // comment 外から呼べる関数なので、アンダーバーはいらないのでは、と思います。
        // そしてライブラリなので、usingを使って、feeParams.validate()　で実行できるようにしてもいいと思います。
        IncentiveLib._validateFeeParams(_feeParams);

        __AccessControl_init();
        __ReentrancyGuard_init();
        __UUPSUpgradeable_init();
        _grantRole(DEFAULT_ADMIN_ROLE, initialOwner);
        _grantRole(FEE_MANAGER_ROLE, initialOwner);

        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        $.underlyingToken = IERC20(_underlyingToken); // comment 不変なので、immutableにしていいと思います 
        $.zerc20 = IzERC20(_zerc20);// comment 不変なので、immutableにしていいと思います。
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
        if (amountOut < minOut) revert SlippageExceeded();
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
        if (amountOut < minOut) revert SlippageExceeded();
    }

    // ---------------------------- Views ----------------------------------

    /// @notice Quotes reward paid for wrapping `amount` at current liquidity.
    /// @param amount Amount of underlying to wrap.
    /// @return reward Reward amount paid from fee surplus.
    function quoteWrapReward(uint256 amount) public view returns (uint256 reward) {
        return _quoteWrapReward(amount, _getLiquidityManagerStorage());
    }

    /// @notice Quotes fee charged for unwrapping `amount` at current liquidity.
    /// @param amount Amount of zERC20 to unwrap.
    /// @return feeAmount Fee charged in underlying units.
    function quoteUnwrapFee(uint256 amount) public view returns (uint256 feeAmount) {
        return _quoteUnwrapFee(amount, _getLiquidityManagerStorage());
    }

    /// @notice Returns the wrapped underlying token.
    function underlyingToken() public view returns (IERC20) {
        return _getLiquidityManagerStorage().underlyingToken;
    }

    /// @notice Returns the zERC20 token minted/burned by this contract.
    function zerc20() public view returns (IzERC20) {
        return _getLiquidityManagerStorage().zerc20;
    }

    /// @notice Returns the incentive curve parameters.
    function feeParams() public view returns (IncentiveLib.FeeParams memory params) {
        params = _getLiquidityManagerStorage().feeParams;
    }

    /// @notice Returns the fee surplus available for rewards and admin withdrawals.
    function feeSurplus() public view returns (uint256) {
        return _getLiquidityManagerStorage().feeSurplus;
    }

    // ---------------------------- Admin ------------------------------------

    /// @notice Updates the incentive curve parameters.
    /// @param params New fee parameters.
    function setFeeParams(IncentiveLib.FeeParams calldata params) external onlyRole(FEE_MANAGER_ROLE) {
        IncentiveLib._validateFeeParams(params);
        _getLiquidityManagerStorage().feeParams = params;
        emit FeeParamsUpdated(params);
    }

    /// @notice Withdraws accumulated fee surplus to the provided address.
    /// @param to Recipient of the withdrawal.
    /// @param amount Amount of underlying to withdraw.
    function withdrawRewards(address to, uint256 amount) external nonReentrant onlyRole(DEFAULT_ADMIN_ROLE) {
        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        if (to == address(0)) revert ZeroReceiver();
        if (amount == 0) revert ZeroAmount();
        if (amount > $.feeSurplus) revert InsufficientRewards();

        $.feeSurplus -= amount;
        if (_isNativeUnderlying($)) {
            (bool success,) = payable(to).call{value: amount}("");
            if (!success) revert UnderlyingSendFailed();
        } else {
            $.underlyingToken.safeTransfer(to, amount);
        }
        emit RewardsWithdrawn(to, amount);
    }

    // ---------------------------- Internal ----------------------------------

    /// @dev Returns the storage pointer for ERC-7201 layout.
    function _getLiquidityManagerStorage() private pure returns (LiquidityManagerStorage storage $) {
        // comment 毎回betes32にする計算を実行するのはコストがかかるので、定数にしてしまっていいと思いますが、どうでしょうか。
        bytes32 slot = SlotDerivation.erc7201Slot("zerc20.storage.liquidityManager");
        assembly {
            $.slot := slot
        }
    }

    // comment immutableな結果になるので、そもそもこの値をimmutableなbool変数に入れてしまってもいいのでは、と思います。どうでしょうか
    function _isNativeUnderlying(LiquidityManagerStorage storage $) private view returns (bool) {
        return address($.underlyingToken) == NATIVE_TOKEN;
    }

    function _underlyingBalance(LiquidityManagerStorage storage $) private view returns (uint256) {
        if (_isNativeUnderlying($)) {
            return address(this).balance;
        }
        return $.underlyingToken.balanceOf(address(this));
    }

    /// @dev Quotes wrapping reward using current token balance and fee surplus.
    function _quoteWrapReward(uint256 amount, LiquidityManagerStorage storage $)
        internal
        view
        returns (uint256 reward)
    {
        uint256 balance = _underlyingBalance($);
        uint256 feeSurplus_ = $.feeSurplus;
        // @note: underflow is unlikely here but possible if balance of underlying token changes externally.
        uint256 liquidity = balance >= feeSurplus_ ? balance - feeSurplus_ : 0;
        reward = IncentiveLib.quoteWrapReward($.feeParams, liquidity, feeSurplus_, amount);
    }

    /// @dev Quotes unwrap fee using current token balance and fee surplus.
    function _quoteUnwrapFee(uint256 amount, LiquidityManagerStorage storage $)
        internal
        view
        returns (uint256 feeAmount)
    {
        uint256 balance = _underlyingBalance($);
        uint256 feeSurplus_ = $.feeSurplus;
        // @note: underflow is unlikely here but possible if balance of underlying token changes externally.
        uint256 liquidity = balance >= feeSurplus_ ? balance - feeSurplus_ : 0;
        feeAmount = IncentiveLib.quoteUnwrapFee($.feeParams, liquidity, amount);
    }

    // comment privateでいいと思うのですが、どうでしょうか。
    // ここだけではなく、privateにできるinternalはprivateのほうが隠匿性が高まるので、いいかと思いますが、どうでしょう。
    /// @dev Internal wrap implementation using pre-deposit liquidity for reward quotes.
    function _wrap(uint256 amount, address receiver) internal returns (uint256 amountOut) {
        // comment require(bool, customError) の形式にするのがモダンな書き方ですね
        // if文を使うより、Slitherなどの静的解析ツールも理解しやすいロジックになります。
        // audit会社にもよりますが、if文の数でお金が変わることもあります。(伝聞)
        if (amount == 0) revert ZeroAmount();
        if (receiver == address(0)) revert ZeroReceiver();

        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        bool isNative = _isNativeUnderlying($);
        /// @note: for native, msg.value is already in address(this).balance; for ERC20, balance updates after transferFrom.
        uint256 balanceBefore = isNative ? address(this).balance - msg.value : _underlyingBalance($);
        uint256 received;

        if (isNative) {
            if (msg.value != amount) revert InvalidMsgValue(amount, msg.value);
            received = msg.value;
        } else {
            if (msg.value != 0) revert InvalidMsgValue(0, msg.value);
            IERC20 underlying = $.underlyingToken;
            underlying.safeTransferFrom(msg.sender, address(this), amount);
            received = _underlyingBalance($) - balanceBefore;
        }
        if (received == 0) revert UnderlyingPullFailed();

        uint256 feeSurplus_ = $.feeSurplus;
        // Keep reward calculation aligned with pre-deposit liquidity.
        uint256 liquidityBefore = balanceBefore >= feeSurplus_ ? balanceBefore - feeSurplus_ : 0;
        // comment ここもusingを使って$.feeParams.quoteWrapReward(...)で実行できるようにすれば、引数減らせますね
        uint256 reward = IncentiveLib.quoteWrapReward($.feeParams, liquidityBefore, feeSurplus_, received);

        if (reward > 0) $.feeSurplus -= reward;
        amountOut = received + reward;
        $.zerc20.mint(receiver, amountOut);
        emit Wrapped(msg.sender, receiver, amountOut, reward);
    }

    /// @dev Internal unwrap implementation that burns zERC20 and transfers underlying.
    function _unwrap(uint256 amount, address receiver) internal returns (uint256 amountOut) {
        if (amount == 0) revert ZeroAmount();
        if (receiver == address(0)) revert ZeroReceiver();

        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        uint256 feeAmount = _quoteUnwrapFee(amount, $);
        amountOut = amount - feeAmount;

        $.zerc20.burn(msg.sender, amount);
        if (amountOut > 0) {
            if (_isNativeUnderlying($)) {
                if (address(this).balance < amountOut) revert InsufficientLiquidity();
                (bool success,) = payable(receiver).call{value: amountOut}("");
                if (!success) revert UnderlyingSendFailed();
            } else {
                IERC20 underlying = $.underlyingToken;
                if (underlying.balanceOf(address(this)) < amountOut) revert InsufficientLiquidity();
                underlying.safeTransfer(receiver, amountOut);
            }
        }
        if (feeAmount > 0) $.feeSurplus += feeAmount;

        emit Unwrapped(msg.sender, receiver, amountOut, feeAmount);
    }

    /// @dev Restricts upgrade authorization to admins.
    function _authorizeUpgrade(address) internal override onlyRole(DEFAULT_ADMIN_ROLE) {}

    receive() external payable nonReentrant {
        LiquidityManagerStorage storage $ = _getLiquidityManagerStorage();
        if (!_isNativeUnderlying($)) revert NativeTokenNotSupported();
        _wrap(msg.value, msg.sender);
    }
}
