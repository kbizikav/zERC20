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
import {SelfCall} from "../utils/SelfCall.sol";

/// @notice Receives zERC20 (typically via OFT), unwraps through LiquidityManager, and forwards the underlying token through Stargate.
contract Adaptor2 is ReentrancyGuard, SelfCall, ILayerZeroComposer {
    using OptionsBuilder for bytes;

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
    error AmountMismatch(uint256 expected, uint256 actual);
    error OutputTooLow(uint256 amountOut, uint256 amountMinOut);
    error TransferFailed();
    error ApproveFailed();
    error InvalidComposeCaller();
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

    mapping(address => uint256) public underlingTokenBalances;
    mapping(address => uint256) public zerc20Balances;
    mapping(address => uint256) public nativeBalances;

    event UnwrapAndBridge(address indexed caller, uint256 amountIn, uint256 amountOut, address receiver, uint32 dstEid);
    event BridgeZerc20(address indexed to, uint32 indexed dstEid, uint256 amountReturned);
    event BridgeUnderlyingToken(
        address indexed user, address indexed to, uint32 indexed dstEid, uint256 amountOut, uint256 nativeFeeUsed
    );

    event DecodeBridgeRequestFailed(bytes message);
    event QuoteFailed(uint256 amount, BridgeRequest request);

    /// @param _liquidityManager LiquidityManager that wraps/unwraps the zERC20.
    /// @param _stargate Stargate endpoint used for bridging the underlying token.
    constructor(address _liquidityManager, address _stargate) {
        if (_liquidityManager == address(0)) revert ZeroAddress();
        if (_stargate == address(0)) revert ZeroAddress();
        LIQUIDITY_MANAGER = ILiquidityManager(_liquidityManager);
        UNDERLYING_TOKEN = IERC20(LIQUIDITY_MANAGER.underlyingToken());
        ZERC20 = IzERC20(address(LIQUIDITY_MANAGER.zerc20()));
        STARGATE = IStargate(_stargate);
    }

    function lzCompose(address _from, bytes32, bytes calldata _message, address, bytes calldata)
        external
        payable
        override
    {
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
        } catch {
            emit DecodeBridgeRequestFailed(_message);
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
        uint256 amountAfterUnwrap = amount - tokenUnwrapFee;
        SendParam memory sendParam = SendParam({
            dstEid: request.dstEid,
            to: _toBytes32(request.to),
            amountLD: amountAfterUnwrap,
            minAmountLD: 0, // use zero to avoid revert
            extraOptions: request.extraOptions,
            composeMsg: bytes(""),
            oftCmd: bytes("")
        });
        MessagingFee memory feeQuote = STARGATE.quoteSend(sendParam, false);
        (,, OFTReceipt memory receipt) = STARGATE.quoteOFT(sendParam);
        uint256 tokenBridgeFee = amountAfterUnwrap - receipt.amountReceivedLD;
        quote = FeeQuote({
            tokenUnwrapFee: tokenUnwrapFee,
            nativeBridgeFee: feeQuote.nativeFee,
            tokenBridgeFee: tokenBridgeFee
        });
    }

    /// @notice Withdraws previously deposited tokens from the adaptor.
    function withdraw(address token, uint256 amount) external nonReentrant {
        require(amount > 0, ZeroAmount());
        if (token == address(UNDERLYING_TOKEN)) {
            _debitUnderlyingBalance(msg.sender, amount);
            if (!UNDERLYING_TOKEN.transfer(msg.sender, amount)) revert TransferFailed();
        } else if (token == address(ZERC20)) {
            _debitZerc20Balance(msg.sender, amount);
            if (!ZERC20.transfer(msg.sender, amount)) revert TransferFailed();
        } else if (token == NATIVE_TOKEN) {
            _debitNativeBalance(msg.sender, amount);
            (bool success,) = payable(msg.sender).call{value: amount}("");
            if (!success) revert TransferFailed();
        } else {
            revert InvalidToken();
        }
    }

    // ------------------------- External Self-Calls ----------------------------

    function decodeBridgeRequest(bytes calldata _message) external pure returns (BridgeRequest memory request) {
        request = abi.decode(OFTComposeMsgCodec.composeMsg(_message), (BridgeRequest));
    }

    function unwrapSelf(address user, uint256 amount, uint256 amountMinOut)
        external
        onlySelfCall
        returns (uint256 amountOut)
    {
        amountOut = _unwrap(user, amount, amountMinOut);
    }

    function bridgeUnderlyingTokenSelf(
        address user,
        uint256 amount,
        uint256 nativeBridgeFee,
        BridgeRequest calldata request
    ) external onlySelfCall returns (uint256 amountOut) {
        amountOut = _bridgeUnderlyingToken(user, amount, nativeBridgeFee, request);
    }

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
        } catch {
            emit QuoteFailed(zerc20Amount, request);
            return;
        }

        // check min output
        if (zerc20Amount <= quote.tokenUnwrapFee + quote.tokenBridgeFee) {
            try this.bridgeZerc20Self(request.dstEid, user, request.to, zerc20Amount) {} catch {}
            return;
        }

        uint256 amountOutput = zerc20Amount - quote.tokenUnwrapFee - quote.tokenBridgeFee;
        if (amountOutput < request.minAmountOut) {
            // send back zERC20 to user on slippage exceed
            try this.bridgeZerc20Self(request.dstEid, user, request.to, zerc20Amount) {} catch {}
            return;
        }

        // unwrap
        uint256 underlyingTokenAmount;
        try this.unwrapSelf(user, zerc20Amount, quote.tokenUnwrapFee) returns (uint256 amountOut_) {
            underlyingTokenAmount = amountOut_;
        } catch {
            // this is extremely unlikely to happen since we have already quoted the unwrap fee
            return;
        }

        // bridge
        uint256 amountOut;
        try this.bridgeUnderlyingTokenSelf(user, underlyingTokenAmount, quote.nativeBridgeFee, request) returns (
            uint256 amountOut_
        ) {
            amountOut = amountOut_;
        } catch {
            // this is extremely unlikely to happen since we have already quoted the bridge fee
            return;
        }

        emit UnwrapAndBridge(user, zerc20Amount, amountOut, request.to, request.dstEid);
    }

    function _unwrap(address user, uint256 amount, uint256 amountMinOut) internal returns (uint256 amountOut) {
        _debitZerc20Balance(user, amount);

        uint256 underlyingTokenBalanceBefore = UNDERLYING_TOKEN.balanceOf(address(this));

        // unwrap
        amountOut = LIQUIDITY_MANAGER.unwrap(amount, address(this));
        if (amountOut < amountMinOut) revert OutputTooLow(amountOut, amountMinOut);

        uint256 underlyingTokenBalanceAfter = UNDERLYING_TOKEN.balanceOf(address(this));
        uint256 actualAmountOut = underlyingTokenBalanceAfter - underlyingTokenBalanceBefore;
        // additional safety check
        require(actualAmountOut == amountOut, AmountMismatch(amountOut, actualAmountOut));

        // add underlying token balance
        underlingTokenBalances[user] += amountOut;
    }

    function _bridgeUnderlyingToken(
        address user,
        uint256 amount,
        uint256 nativeBridgeFee,
        BridgeRequest calldata request
    ) internal returns (uint256 amountOut) {
        _debitNativeBalance(user, nativeBridgeFee);
        _debitUnderlyingBalance(user, amount);

        SendParam memory sendParam = SendParam({
            dstEid: request.dstEid,
            to: _toBytes32(request.to),
            amountLD: amount,
            minAmountLD: request.minAmountOut,
            extraOptions: request.extraOptions,
            composeMsg: request.composeMsg,
            oftCmd: request.oftCmd
        });
        if (!UNDERLYING_TOKEN.approve(address(STARGATE), amount)) revert ApproveFailed();
        MessagingFee memory fee = MessagingFee({nativeFee: nativeBridgeFee, lzTokenFee: 0});

        uint256 actualNativeFee;
        {
            uint256 nativeBalanceBefore = address(this).balance;
            (, OFTReceipt memory oftReceipt,) =
                STARGATE.sendToken{value: nativeBridgeFee}(sendParam, fee, address(this));
            amountOut = oftReceipt.amountReceivedLD;
            actualNativeFee = nativeBalanceBefore - address(this).balance;
        }

        // refund any surplus native fee back to user if applicable
        // usually shouldn't happen unless there is a change in Stargate fee structure
        if (nativeBridgeFee > actualNativeFee) {
            nativeBalances[user] += nativeBridgeFee - actualNativeFee;
        }
        if (amountOut < request.minAmountOut) revert OutputTooLow(amountOut, request.minAmountOut);
        emit BridgeUnderlyingToken(user, request.to, request.dstEid, amountOut, actualNativeFee);
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

        uint256 nativeBalanceBefore = address(this).balance;
        ZERC20.send{value: nativeFee}(sendParam, returnFeeQuote, address(this));
        uint256 nativeBalanceAfter = address(this).balance;
        uint256 actualNativeFee = nativeBalanceBefore - nativeBalanceAfter;
        // refund any surplus native fee back to user if applicable
        if (nativeFee > actualNativeFee) {
            uint256 refundAmount = nativeFee - actualNativeFee;
            nativeBalances[user] += refundAmount;
        }
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

    function _debitZerc20Balance(address user, uint256 amount) internal {
        uint256 userBalance = zerc20Balances[user];
        if (userBalance < amount) revert InsufficientZerc20Balance();
        zerc20Balances[user] = userBalance - amount;
    }

    function _toBytes32(address a) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(a)));
    }
}
