// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ILiquidityManager} from "./interfaces/ILiquidityManager.sol";
import {IStargate, Ticket} from "./interfaces/IStargate.sol";
import {IzERC20} from "./interfaces/IzERC20.sol";
import {
    SendParam,
    MessagingFee,
    OFTReceipt,
    MessagingReceipt
} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oft/interfaces/IOFT.sol";
import {OFTComposeMsgCodec} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oft/libs/OFTComposeMsgCodec.sol";
import {ILayerZeroComposer} from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/ILayerZeroComposer.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @notice Receives zERC20 (typically via OFT), unwraps through LiquidityManager, and forwards the underlying token through Stargate.
contract Adaptor is ILayerZeroComposer {
    struct FeeQuote {
        uint256 tokenUnwrapFee;
        uint256 nativeBridgeFee;
        uint256 tokenBridgeFee;
    }

    struct BridgeRequest {
        uint32 dstEid;
        bytes extraOptions;
        bytes composeMsg;
        bytes oftCmd;
        address refundAddress;
        address to;
        uint256 minAmountOut; // Minimum bridged tokens expected on the destination chain.
    }

    error ZeroAddress();
    error LzTokenFeeUnsupported();
    error SlippageTooHigh();
    error NativeFeeTooLow();
    error TransferFailed();
    error TokenPullFailed();
    error ApproveFailed();
    error InvalidComposeCaller();
    error InsufficientZerc20();
    error StargateSendFailed();

    ILiquidityManager public immutable LIQUIDITY_MANAGER;
    IERC20 public immutable UNDERLYING_TOKEN;
    IzERC20 public immutable ZERC20;
    IStargate public immutable STARGATE;

    event StargateSendFailure(
        uint256 nativeFee,
        SendParam sendParam,
        MessagingFee fee,
        address refundAddress
    );
    event UnwrapAndBridge(address indexed caller, uint256 amountIn, uint256 amountOut, address receiver, uint32 dstEid);
    event ReturnZerc20(address indexed to, uint32 indexed dstEid, uint256 amountReturned);

    constructor(address _liquidityManager, address _stargate) {
        require(_liquidityManager != address(0), ZeroAddress());
        require(_stargate != address(0), ZeroAddress());
        LIQUIDITY_MANAGER = ILiquidityManager(_liquidityManager);
        UNDERLYING_TOKEN = IERC20(LIQUIDITY_MANAGER.underlyingToken());
        ZERC20 = IzERC20(address(LIQUIDITY_MANAGER.zerc20()));
        STARGATE = IStargate(_stargate);
    }

    // ---------------------------- User flows --------------------------------

    function unwrapAndBridge(uint256 amount, BridgeRequest calldata request)
        external
        payable
        returns (uint256 amountOut)
    {
        require(ZERC20.transferFrom(msg.sender, address(this), amount), TokenPullFailed());
        if (ZERC20.balanceOf(address(this)) < amount) revert InsufficientZerc20();
        FeeQuote memory quote = _quoteFee(amount, request);
        bool success;
        (amountOut, success) = _unwrapAndBridge(amount, request, quote);
        if (!success) revert StargateSendFailed();
    }

    function lzCompose(address _from, bytes32, bytes calldata _message, address, bytes calldata)
        external
        payable
        override
    {
        if (_from != address(ZERC20)) revert InvalidComposeCaller();
        BridgeRequest memory request = abi.decode(OFTComposeMsgCodec.composeMsg(_message), (BridgeRequest));
        uint256 amount = OFTComposeMsgCodec.amountLD(_message);
        if (ZERC20.balanceOf(address(this)) < amount) revert InsufficientZerc20();

        FeeQuote memory quote = _quoteFee(amount, request);
        /// @dev This never overflows because of quoteFee logic
        uint256 expectedAmountOut = amount - quote.tokenUnwrapFee - quote.tokenBridgeFee;

        if (expectedAmountOut < request.minAmountOut) {
            _returnZerc20(amount, request);
            return;
        }
        (uint256 bridgedOrUnwrapped, bool success) = _unwrapAndBridge(amount, request, quote);
        if (success) return;

        uint256 wrappedAmount = _wrapBack(bridgedOrUnwrapped);
        _returnZerc20(wrappedAmount, request);
    }

    // ---------------------------- Core flows --------------------------------

    function _unwrapAndBridge(
        uint256 amount,
        BridgeRequest memory request,
        FeeQuote memory quote
    ) internal returns (uint256 amountOut, bool success) {
        if (msg.value < quote.nativeBridgeFee) revert NativeFeeTooLow();
        uint256 amountAfterUnwrap = LIQUIDITY_MANAGER.unwrap(amount, address(this));
        (amountOut, success) = _bridge(amountAfterUnwrap, request, quote);

        if (success) {
            emit UnwrapAndBridge(msg.sender, amount, amountOut, request.to, request.dstEid);
            return (amountOut, success);
        }

        amountOut = amountAfterUnwrap; // underlying amount available for wrap-back
    }

    function _bridge(uint256 amount, BridgeRequest memory request, FeeQuote memory quote)
        internal
        returns (uint256 amountOut, bool success)
    {
        uint256 nativeFee = quote.nativeBridgeFee;
        if (msg.value < nativeFee) revert NativeFeeTooLow();
        uint256 refundAmount = msg.value - nativeFee;
        uint256 minAmount = request.minAmountOut;
        if (minAmount > amount) revert SlippageTooHigh();
        SendParam memory sendParam = _buildSendParam(amount, minAmount, request);
        IERC20 erc20 = UNDERLYING_TOKEN;
        if (!erc20.approve(address(STARGATE), amount)) revert ApproveFailed();
        MessagingFee memory fee = MessagingFee({nativeFee: nativeFee, lzTokenFee: 0});

        /// @dev Stargate reverts if msg.value differs from the quoted native fee, so pass the exact quote and refund any surplus.
        try STARGATE.sendToken{value: nativeFee}(sendParam, fee, request.refundAddress) returns (
            MessagingReceipt memory,
            OFTReceipt memory oftReceipt,
            Ticket memory
        ) {
            amountOut = oftReceipt.amountReceivedLD;
            success = true;
        } catch (bytes memory) {
            success = false;
            amountOut = 0;
            emit StargateSendFailure(nativeFee, sendParam, fee, request.refundAddress);
        }

        if (success && refundAmount > 0) {
            (bool refundSuccess, ) = payable(request.refundAddress).call{value: refundAmount}("");
            if (!refundSuccess) revert TransferFailed();
        }
    }

    function _wrapBack(uint256 amount) internal returns (uint256 wrappedAmount) {
        if (!UNDERLYING_TOKEN.approve(address(LIQUIDITY_MANAGER), amount)) revert ApproveFailed();
        wrappedAmount = LIQUIDITY_MANAGER.wrap(amount, address(this));
    }

    function _returnZerc20(uint256 amount, BridgeRequest memory request) internal {
        SendParam memory sendParam = SendParam({
            dstEid: request.dstEid,
            to: _toBytes32(request.to),
            amountLD: amount,
            minAmountLD: amount,
            extraOptions: request.extraOptions,
            composeMsg: bytes(""),
            oftCmd: bytes("")
        });
        MessagingFee memory returnFeeQuote = ZERC20.quoteSend(sendParam, false);
        uint256 nativeFee = returnFeeQuote.nativeFee;
        if (msg.value < nativeFee) revert NativeFeeTooLow();
        ZERC20.send{value: msg.value}(sendParam, returnFeeQuote, request.refundAddress);
        emit ReturnZerc20(request.to, request.dstEid, amount);
    }

    // ---------------------------- Quoting -----------------------------------
    function quoteFee(uint256 amount, BridgeRequest memory request)
        external
        view
        returns (FeeQuote memory quote)
    {
        return _quoteFee(amount, request);
    }

    function _quoteFee(uint256 amount, BridgeRequest memory request)
        internal
        view
        returns (FeeQuote memory quote)
    {
        uint256 tokenUnwrapFee = LIQUIDITY_MANAGER.quoteUnwrap(amount);
        uint256 amountAfterUnwrap = amount - tokenUnwrapFee;
        SendParam memory sendParam = _buildSendParam(amountAfterUnwrap, 0, request);
        MessagingFee memory feeQuote = _quoteSend(sendParam);
        uint256 amountReceived = _quoteAmountReceived(sendParam);
        uint256 tokenBridgeFee = amountAfterUnwrap > amountReceived ? amountAfterUnwrap - amountReceived : 0;
        quote =
            FeeQuote({
                tokenUnwrapFee: tokenUnwrapFee,
                nativeBridgeFee: feeQuote.nativeFee,
                tokenBridgeFee: tokenBridgeFee
            });
    }

    function _quoteSend(SendParam memory sendParam) internal view returns (MessagingFee memory fee) {
        fee = STARGATE.quoteSend(sendParam, false);
        if (fee.lzTokenFee > 0) revert LzTokenFeeUnsupported();
    }

    function _quoteAmountReceived(SendParam memory sendParam) internal view returns (uint256 amountReceived) {
        (,, OFTReceipt memory receipt) = STARGATE.quoteOFT(sendParam);
        amountReceived = receipt.amountReceivedLD;
    }

    // ---------------------------- Builders & utils --------------------------

    function _buildSendParam(uint256 amount, uint256 minAmount, BridgeRequest memory request)
        internal
        pure
        returns (SendParam memory sendParam)
    {
        sendParam = SendParam({
            dstEid: request.dstEid,
            to: _toBytes32(request.to),
            amountLD: amount,
            minAmountLD: minAmount,
            extraOptions: request.extraOptions,
            composeMsg: request.composeMsg,
            oftCmd: request.oftCmd
        });
    }

    function _toBytes32(address a) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(a)));
    }
}
