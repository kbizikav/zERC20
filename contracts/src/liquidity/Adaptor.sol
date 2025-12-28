// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ILiquidityManager} from "../interfaces/ILiquidityManager.sol";
import {IStargate} from "../interfaces/IStargate.sol";
import {IzERC20} from "../interfaces/IzERC20.sol";
import {SendParam, MessagingFee, OFTReceipt} from "@layerzerolabs/oft-evm/contracts/interfaces/IOFT.sol";
import {OptionsBuilder} from "@layerzerolabs/oapp-evm/contracts/oapp/libs/OptionsBuilder.sol";
import {OFTComposeMsgCodec} from "@layerzerolabs/oft-evm/contracts/libs/OFTComposeMsgCodec.sol";
import {ILayerZeroComposer} from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/ILayerZeroComposer.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {SelfCall} from "../utils/SelfCall.sol";

/// @title Stargate adaptor for zERC20 unwrap + bridge flows.
/// @notice Receives zERC20 (typically via OFT), unwraps through LiquidityManager, and forwards the underlying token through Stargate.
contract Adaptor is ReentrancyGuard, SelfCall, ILayerZeroComposer {
    using OptionsBuilder for bytes;
    using SafeERC20 for IERC20;

    /// @notice Breaks down the expected fees for unwrapping and bridging.
    struct FeeQuote {
        uint256 tokenUnwrapFee;
        uint256 nativeBridgeFee;
        uint256 tokenBridgeFee;
    }

    /// @notice Parameters for forwarding unwrapped tokens through Stargate.
    struct BridgeRequest {
        uint32 dstEid;
        address to;
        uint256 minAmountOut; // Minimum bridged tokens expected on the destination chain.
        bytes extraOptions;
        bytes composeMsg;
        bytes oftCmd;
    }

    error ZeroAddress();
    error ZeroAmount();
    error InvalidToken();
    error UnderlyingTokenMismatch(address expected, address actual);
    error AmountMismatch(uint256 expected, uint256 actual);
    error OutputTooLow(uint256 amountOut, uint256 amountMinOut);
    error TransferFailed();
    error ApproveFailed();
    error InvalidComposeCaller();
    error InvalidComposeSender();
    error InsufficientZerc20Balance();
    error InsufficientUnderlyingBalance();
    error InsufficientNativeBalance();

    uint128 internal constant RETURN_LZ_RECEIVE_GAS = 500_000;

    /// @dev erc-7528 native token address convention
    address constant NATIVE_TOKEN = 0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE;

    ILiquidityManager public immutable LIQUIDITY_MANAGER;
    IERC20 public immutable UNDERLYING_TOKEN;
    IzERC20 public immutable ZERC20;
    IStargate public immutable STARGATE;
    address public immutable LZ_ENDPOINT;

    mapping(address => uint256) public underlingTokenBalances;
    mapping(address => uint256) public zerc20Balances;
    mapping(address => uint256) public nativeBalances;
    address private _refundRecipient;
    uint256 private _refundAccrued;
    bool private _refundActive;

    event UnwrapAndBridge(address indexed caller, uint256 amountIn, uint256 amountOut, address receiver, uint32 dstEid);
    event BridgeZerc20(address indexed to, uint32 indexed dstEid, uint256 amountReturned);
    event BridgeUnderlyingToken(
        address indexed user, address indexed to, uint32 indexed dstEid, uint256 amountOut, uint256 nativeFeeUsed
    );
    event Unwrap(address indexed user, uint256 amountIn, uint256 amountOut);
    event UnwrapFailed(address indexed user, uint256 amount, uint256 minAmountOut, bytes revertData);
    event BridgeUnderlyingTokenFailed(
        address indexed user,
        address indexed to,
        uint32 indexed dstEid,
        uint256 amount,
        uint256 nativeBridgeFee,
        uint256 minAmountOut,
        bytes revertData
    );
    event BridgeZerc20Failed(
        address indexed user, address indexed to, uint32 indexed dstEid, uint256 amount, bytes revertData
    );

    event DecodeBridgeRequestFailed(bytes message, bytes revertData);
    event QuoteFailed(uint256 amount, BridgeRequest request, bytes revertData);
    event NativeDeposit(address indexed sender, uint256 amount);

    /// @param _liquidityManager LiquidityManager that wraps/unwraps the zERC20.
    /// @param _stargate Stargate endpoint used for bridging the underlying token.
    /// @param _lzEndpoint LayerZero endpoint that invokes lzCompose.
    constructor(address _liquidityManager, address _stargate, address _lzEndpoint) {
        if (_liquidityManager == address(0)) revert ZeroAddress();
        if (_stargate == address(0)) revert ZeroAddress();
        if (_lzEndpoint == address(0)) revert ZeroAddress();
        LIQUIDITY_MANAGER = ILiquidityManager(_liquidityManager);
        UNDERLYING_TOKEN = IERC20(LIQUIDITY_MANAGER.underlyingToken());
        ZERC20 = IzERC20(address(LIQUIDITY_MANAGER.zerc20()));
        STARGATE = IStargate(_stargate);
        LZ_ENDPOINT = _lzEndpoint;
        address stargateToken = STARGATE.token();
        if (_isNativeUnderlying()) {
            if (stargateToken != NATIVE_TOKEN && stargateToken != address(0)) {
                revert UnderlyingTokenMismatch(address(UNDERLYING_TOKEN), stargateToken);
            }
        } else if (stargateToken != address(UNDERLYING_TOKEN)) {
            revert UnderlyingTokenMismatch(address(UNDERLYING_TOKEN), stargateToken);
        }
    }

    /// @notice Handles LayerZero compose callbacks from the zERC20 to unwrap and bridge.
    /// @dev Valid　compose messages should not revert to avoid trapping cross-chain flows.
    /// @param _from Compose sender on the source chain (must be the zERC20 instance).
    /// @param _message Encoded compose payload carrying the BridgeRequest.
    function lzCompose(address _from, bytes32, bytes calldata _message, address, bytes calldata)
        external
        payable
        override
        nonReentrant
    {
        if (msg.sender != LZ_ENDPOINT) revert InvalidComposeSender();
        if (_from != address(ZERC20)) revert InvalidComposeCaller();

        bytes32 composeFromBytes = OFTComposeMsgCodec.composeFrom(_message);
        address user = OFTComposeMsgCodec.bytes32ToAddress(composeFromBytes);
        uint256 zerc20Amount = OFTComposeMsgCodec.amountLD(_message);

        // record zERC20 & native balance
        zerc20Balances[user] += zerc20Amount;
        nativeBalances[user] += msg.value;

        // Decode BridgeRequest using try-catch to avoid revert on malformed message
        BridgeRequest memory request;
        try this.decodeBridgeRequest(_message) returns (BridgeRequest memory request_) {
            request = request_;
        } catch (bytes memory revertData) {
            emit DecodeBridgeRequestFailed(_message, revertData);
            return;
        }

        if (zerc20Amount == 0) {
            return;
        }

        _unwrapAndBridge(user, zerc20Amount, request);
    }

    // ---------------------------- Operations ---------------------------------

    /// @notice Returns fee estimates for unwrapping and bridging the provided amount.
    /// @param amount zERC20 amount to unwrap.
    /// @param request Bridge instructions used to derive messaging fees.
    /// @return quote Fee breakdown (unwrap fee, native bridge fee, token bridge fee).
    function quoteFee(uint256 amount, BridgeRequest memory request) external view returns (FeeQuote memory quote) {
        uint256 tokenUnwrapFee = LIQUIDITY_MANAGER.quoteUnwrapFee(amount);
        if (tokenUnwrapFee >= amount) {
            // Prevent underflow and short-circuit when unwrap fee consumes the amount.
            return FeeQuote({tokenUnwrapFee: tokenUnwrapFee, nativeBridgeFee: 0, tokenBridgeFee: 0});
        }
        uint256 amountAfterUnwrap = amount - tokenUnwrapFee;
        if (amountAfterUnwrap == 0) {
            // Early return to avoid Stargate quote revert on zero amount
            return FeeQuote({tokenUnwrapFee: tokenUnwrapFee, nativeBridgeFee: 0, tokenBridgeFee: 0});
        }
        SendParam memory sendParam = SendParam({
            dstEid: request.dstEid,
            to: _toBytes32(request.to),
            amountLD: amountAfterUnwrap,
            minAmountLD: 0, // set 0 to prevent a revert of the minimum amount
            extraOptions: request.extraOptions,
            composeMsg: request.composeMsg,
            oftCmd: request.oftCmd
        });
        MessagingFee memory feeQuote = STARGATE.quoteSend(sendParam, false);
        (,, OFTReceipt memory receipt) = STARGATE.quoteOFT(sendParam);
        uint256 tokenBridgeFee = 0;
        if (receipt.amountReceivedLD < amountAfterUnwrap) {
            tokenBridgeFee = amountAfterUnwrap - receipt.amountReceivedLD;
        }
        quote = FeeQuote({tokenUnwrapFee: tokenUnwrapFee, nativeBridgeFee: feeQuote.nativeFee, tokenBridgeFee: tokenBridgeFee});
    }

    /// @notice Pulls zERC20 from the caller, unwraps it, and bridges the underlying token per the request.
    /// @param zerc20Amount Amount of zERC20 to unwrap.
    /// @param request Bridge configuration and minimum output expectations.
    function unwrapAndBridge(uint256 zerc20Amount, BridgeRequest calldata request) external payable nonReentrant {
        if (zerc20Amount == 0) revert ZeroAmount();
        address user = msg.sender;

        // pull zERC20 from user
        IERC20(address(ZERC20)).safeTransferFrom(user, address(this), zerc20Amount);

        // record zERC20 & native balance
        zerc20Balances[user] += zerc20Amount;
        nativeBalances[user] += msg.value;

        _unwrapAndBridge(user, zerc20Amount, request);
    }

    /// @notice Withdraws previously deposited tokens from the adaptor.
    function withdraw(address token, uint256 amount) external nonReentrant {
        if (amount == 0) revert ZeroAmount();
        if (token == NATIVE_TOKEN) {
            if (_isNativeUnderlying()) {
                _debitCombinedNativeBalance(msg.sender, amount);
            } else {
                _debitNativeBalance(msg.sender, amount);
            }
            (bool success,) = payable(msg.sender).call{value: amount}("");
            if (!success) revert TransferFailed();
        } else if (token == address(UNDERLYING_TOKEN)) {
            if (_isNativeUnderlying()) revert InvalidToken();
            _debitUnderlyingBalance(msg.sender, amount);
            UNDERLYING_TOKEN.safeTransfer(msg.sender, amount);
        } else if (token == address(ZERC20)) {
            _debitZerc20Balance(msg.sender, amount);
            IERC20(address(ZERC20)).safeTransfer(msg.sender, amount);
        } else {
            revert InvalidToken();
        }
    }

    // ------------------------- External Self-Calls ----------------------------

    /// @notice Decodes a BridgeRequest from a compose payload.
    /// @param _message Compose payload emitted by the zERC20.
    /// @return request Decoded bridge instructions.
    function decodeBridgeRequest(bytes calldata _message) external pure returns (BridgeRequest memory request) {
        request = abi.decode(OFTComposeMsgCodec.composeMsg(_message), (BridgeRequest));
    }

    /// @notice Self-call hook to unwrap previously credited zERC20 on behalf of a user.
    /// @param user Address whose balance is debited.
    /// @param amount zERC20 amount to unwrap.
    /// @param amountMinOut Minimum underlying expected from the unwrap.
    /// @return amountOut Underlying amount returned by the LiquidityManager.
    function unwrapSelf(address user, uint256 amount, uint256 amountMinOut)
        external
        onlySelfCall
        returns (uint256 amountOut)
    {
        amountOut = _unwrap(user, amount, amountMinOut);
    }

    /// @notice Self-call hook to bridge the user's unwrapped underlying tokens through Stargate.
    /// @param user Address whose balances are debited and refunded on surplus fees.
    /// @param amount Amount of underlying tokens to bridge.
    /// @param nativeBridgeFee Native fee budget forwarded to Stargate.
    /// @param request Bridge parameters including destination and minAmountOut.
    /// @return amountOut Amount delivered to the destination chain after Stargate fees.
    function bridgeUnderlyingTokenSelf(
        address user,
        uint256 amount,
        uint256 nativeBridgeFee,
        BridgeRequest calldata request
    ) external onlySelfCall returns (uint256 amountOut) {
        amountOut = _bridgeUnderlyingToken(user, amount, nativeBridgeFee, request);
    }

    /// @notice Self-call hook to return zERC20 back to the destination when unwrap/bridge cannot proceed.
    /// @param dstEid Destination endpoint ID for the OFT send.
    /// @param user Address whose balance is debited and refunded for unused native fee.
    /// @param to Recipient on the destination chain.
    /// @param amount Amount of zERC20 to bridge back.
    function bridgeZerc20Self(uint32 dstEid, address user, address to, uint256 amount) external onlySelfCall {
        _bridgeZerc20(dstEid, user, to, amount);
    }

    // ---------------------- Internal functions --------------------

    function _unwrapAndBridge(address user, uint256 zerc20Amount, BridgeRequest memory request)
        internal
        enableSelfCall
    {
        // quote fees
        FeeQuote memory quote;
        try this.quoteFee(zerc20Amount, request) returns (FeeQuote memory quote_) {
            quote = quote_;
        } catch (bytes memory revertData) {
            emit QuoteFailed(zerc20Amount, request, revertData);
            return;
        }

        // check min output
        if (zerc20Amount <= quote.tokenUnwrapFee + quote.tokenBridgeFee) {
            try this.bridgeZerc20Self(request.dstEid, user, request.to, zerc20Amount) {}
            catch (bytes memory revertData) {
                emit BridgeZerc20Failed(user, request.to, request.dstEid, zerc20Amount, revertData);
            }
            return;
        }

        uint256 amountOutput = zerc20Amount - quote.tokenUnwrapFee - quote.tokenBridgeFee;
        if (amountOutput < request.minAmountOut) {
            // send back zERC20 to user on slippage exceed
            try this.bridgeZerc20Self(request.dstEid, user, request.to, zerc20Amount) {}
            catch (bytes memory revertData) {
                emit BridgeZerc20Failed(user, request.to, request.dstEid, zerc20Amount, revertData);
            }
            return;
        }

        // unwrap
        uint256 underlyingTokenAmount;
        try this.unwrapSelf(user, zerc20Amount, request.minAmountOut) returns (uint256 amountOut_) {
            underlyingTokenAmount = amountOut_;
        } catch (bytes memory revertData) {
            // this is extremely unlikely to happen since we have already quoted the unwrap fee
            emit UnwrapFailed(user, zerc20Amount, request.minAmountOut, revertData);
            return;
        }

        // bridge
        uint256 amountOut;
        try this.bridgeUnderlyingTokenSelf(user, underlyingTokenAmount, quote.nativeBridgeFee, request) returns (
            uint256 amountOut_
        ) {
            amountOut = amountOut_;
        } catch (bytes memory revertData) {
            // this is extremely unlikely to happen since we have already quoted the bridge fee
            emit BridgeUnderlyingTokenFailed(
                user,
                request.to,
                request.dstEid,
                underlyingTokenAmount,
                quote.nativeBridgeFee,
                request.minAmountOut,
                revertData
            );
            return;
        }

        emit UnwrapAndBridge(user, zerc20Amount, amountOut, request.to, request.dstEid);
    }

    function _unwrap(address user, uint256 amount, uint256 amountMinOut) internal returns (uint256 amountOut) {
        _debitZerc20Balance(user, amount);

        uint256 underlyingTokenBalanceBefore = _underlyingBalance();

        // unwrap
        amountOut = LIQUIDITY_MANAGER.unwrap(amount, address(this));
        if (amountOut < amountMinOut) revert OutputTooLow(amountOut, amountMinOut);

        uint256 underlyingTokenBalanceAfter = _underlyingBalance();
        uint256 actualAmountOut = underlyingTokenBalanceAfter - underlyingTokenBalanceBefore;
        if (actualAmountOut == 0) revert ZeroAmount();
        if (actualAmountOut < amountMinOut) revert OutputTooLow(actualAmountOut, amountMinOut);
        // Disallow balance increases unrelated to unwrap (e.g. rebases/airdrops).
        if (actualAmountOut > amountOut) revert AmountMismatch(amountOut, actualAmountOut);

        amountOut = actualAmountOut;
        // add underlying token balance
        underlingTokenBalances[user] += amountOut;
        emit Unwrap(user, amount, amountOut);
    }

    function _bridgeUnderlyingToken(
        address user,
        uint256 amount,
        uint256 nativeBridgeFee,
        BridgeRequest calldata request
    ) internal returns (uint256 amountOut) {
        _debitNativeBalance(user, nativeBridgeFee);
        _debitUnderlyingBalance(user, amount);

        uint256 actualNativeFee;
        _startRefundTracking(user);
        (amountOut, actualNativeFee) = _sendUnderlyingToken(amount, nativeBridgeFee, request);
        // Refund any surplus native fee back to user if applicable.
        // Usually shouldn't happen unless there is a change in Stargate fee structure.
        uint256 refundDue = 0;
        if (nativeBridgeFee > actualNativeFee) {
            refundDue = nativeBridgeFee - actualNativeFee;
        }
        if (refundDue > _refundAccrued) {
            nativeBalances[user] += refundDue - _refundAccrued;
        }
        _stopRefundTracking();
        emit BridgeUnderlyingToken(user, request.to, request.dstEid, amountOut, actualNativeFee);
    }

    function _sendUnderlyingToken(uint256 amount, uint256 nativeBridgeFee, BridgeRequest calldata request)
        internal
        returns (uint256 amountOut, uint256 actualNativeFee)
    {
        bool isNative = _isNativeUnderlying();
        SendParam memory sendParam = SendParam({
            dstEid: request.dstEid,
            to: _toBytes32(request.to),
            amountLD: amount,
            minAmountLD: request.minAmountOut,
            extraOptions: request.extraOptions,
            composeMsg: request.composeMsg,
            oftCmd: request.oftCmd
        });
        _ensureAllowance(UNDERLYING_TOKEN, address(STARGATE), amount);
        MessagingFee memory fee = MessagingFee({nativeFee: nativeBridgeFee, lzTokenFee: 0});

        uint256 sendValue = nativeBridgeFee;
        if (isNative) {
            sendValue += amount;
        }
        uint256 nativeBalanceBefore = address(this).balance;
        (, OFTReceipt memory oftReceipt,) = STARGATE.sendToken{value: sendValue}(sendParam, fee, address(this));
        amountOut = oftReceipt.amountReceivedLD;
        uint256 nativeBalanceAfter = address(this).balance;
        uint256 totalSpent = nativeBalanceBefore - nativeBalanceAfter;
        actualNativeFee = totalSpent;
        if (isNative) {
            if (totalSpent < amount) revert AmountMismatch(amount, totalSpent);
            actualNativeFee = totalSpent - amount;
        }
    }

    function _bridgeZerc20(uint32 dstEid, address user, address to, uint256 amount) internal {
        _debitZerc20Balance(user, amount);
        bytes memory extraOptions = OptionsBuilder.newOptions().addExecutorLzReceiveOption(RETURN_LZ_RECEIVE_GAS, 0);
        SendParam memory sendParam = SendParam({
            dstEid: dstEid,
            to: _toBytes32(to),
            amountLD: amount,
            minAmountLD: 0, // to avoid a revert due to dust removal
            extraOptions: extraOptions,
            composeMsg: bytes(""),
            oftCmd: bytes("")
        });
        MessagingFee memory returnFeeQuote = ZERC20.quoteSend(sendParam, false);
        uint256 nativeFee = returnFeeQuote.nativeFee;

        _debitNativeBalance(user, nativeFee);

        _startRefundTracking(user);
        uint256 nativeBalanceBefore = address(this).balance;
        ZERC20.send{value: nativeFee}(sendParam, returnFeeQuote, address(this));
        uint256 nativeBalanceAfter = address(this).balance;
        uint256 actualNativeFee = nativeBalanceBefore - nativeBalanceAfter;
        // Refund any surplus native fee back to user if applicable.
        uint256 refundDue = 0;
        if (nativeFee > actualNativeFee) {
            refundDue = nativeFee - actualNativeFee;
        }
        if (refundDue > _refundAccrued) {
            nativeBalances[user] += refundDue - _refundAccrued;
        }
        _stopRefundTracking();
        emit BridgeZerc20(to, dstEid, amount);
    }

    function _debitNativeBalance(address user, uint256 nativeBridgeFee) internal {
        uint256 userNativeBalance = nativeBalances[user];
        if (userNativeBalance < nativeBridgeFee) revert InsufficientNativeBalance();
        nativeBalances[user] = userNativeBalance - nativeBridgeFee;
    }

    function _debitUnderlyingBalance(address user, uint256 amount) internal {
        uint256 userUnderlyingBalance = underlingTokenBalances[user];
        if (userUnderlyingBalance < amount) revert InsufficientUnderlyingBalance();
        underlingTokenBalances[user] = userUnderlyingBalance - amount;
    }

    function _debitCombinedNativeBalance(address user, uint256 amount) internal {
        uint256 userUnderlyingBalance = underlingTokenBalances[user];
        uint256 userNativeBalance = nativeBalances[user];
        if (userUnderlyingBalance + userNativeBalance < amount) revert InsufficientNativeBalance();
        if (userUnderlyingBalance >= amount) {
            underlingTokenBalances[user] = userUnderlyingBalance - amount;
            return;
        }
        underlingTokenBalances[user] = 0;
        nativeBalances[user] = userNativeBalance - (amount - userUnderlyingBalance);
    }

    function _debitZerc20Balance(address user, uint256 amount) internal {
        uint256 userBalance = zerc20Balances[user];
        if (userBalance < amount) revert InsufficientZerc20Balance();
        zerc20Balances[user] = userBalance - amount;
    }

    function _toBytes32(address a) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(a)));
    }

    function _ensureAllowance(IERC20 token, address spender, uint256 amount) internal {
        if (amount == 0) return;
        if (_isNativeUnderlying()) return;
        uint256 currentAllowance = token.allowance(address(this), spender);
        if (currentAllowance >= amount) return;
        token.forceApprove(spender, amount);
    }

    function _startRefundTracking(address user) internal {
        _refundActive = true;
        _refundRecipient = user;
        _refundAccrued = 0;
    }

    function _stopRefundTracking() internal {
        _refundActive = false;
        _refundRecipient = address(0);
        _refundAccrued = 0;
    }

    function _isNativeUnderlying() internal view returns (bool) {
        return address(UNDERLYING_TOKEN) == NATIVE_TOKEN;
    }

    function _underlyingBalance() internal view returns (uint256) {
        if (_isNativeUnderlying()) {
            return address(this).balance;
        }
        return UNDERLYING_TOKEN.balanceOf(address(this));
    }

    /// @notice Accepts native refunds returned by OFT/Stargate send calls.
    receive() external payable {
        if (msg.sender == address(LIQUIDITY_MANAGER)) return;
        if (_refundActive) {
            nativeBalances[_refundRecipient] += msg.value;
            _refundAccrued += msg.value;
            emit NativeDeposit(_refundRecipient, msg.value);
            return;
        }
        if (msg.sender == LZ_ENDPOINT || msg.sender == address(STARGATE)) return;
        nativeBalances[msg.sender] += msg.value;
        emit NativeDeposit(msg.sender, msg.value);
    }
}
