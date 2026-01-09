// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {ILiquidityManager} from "../interfaces/ILiquidityManager.sol";
import {IStargate} from "../interfaces/IStargate.sol";
import {IzERC20} from "../interfaces/IzERC20.sol";
import {SendParam, MessagingFee, OFTReceipt} from "@layerzerolabs/oft-evm/contracts/interfaces/IOFT.sol";
import {OptionsBuilder} from "@layerzerolabs/oapp-evm/contracts/oapp/libs/OptionsBuilder.sol";
import {OFTComposeMsgCodec} from "@layerzerolabs/oft-evm/contracts/libs/OFTComposeMsgCodec.sol";
import {OFTMsgCodec} from "@layerzerolabs/oft-evm/contracts/libs/OFTMsgCodec.sol";
import {ILayerZeroComposer} from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/ILayerZeroComposer.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {ReentrancyGuardUpgradeable} from "@openzeppelin/contracts-upgradeable/security/ReentrancyGuardUpgradeable.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {SelfCall} from "../utils/SelfCall.sol";

/// @title Stargate adaptor for zERC20 unwrap + bridge flows.
/// @notice Receives zERC20 (typically via OFT), unwraps through LiquidityManager, and forwards the underlying token through Stargate.
contract Adaptor is UUPSUpgradeable, OwnableUpgradeable, ReentrancyGuardUpgradeable, SelfCall, ILayerZeroComposer {
    using OFTMsgCodec for address;
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
    error InvalidDstEid();
    error LiquidityManagerMismatch(address expected, address actual);
    error StargateMismatch(address expected, address actual);
    error LzEndpointMismatch(address expected, address actual);
    error Zerc20Mismatch(address expected, address actual);
    error InsufficientZerc20Balance();
    error InsufficientUnderlyingBalance();
    error InsufficientNativeBalance();

    uint128 private constant RETURN_LZ_RECEIVE_GAS = 500_000;
    uint8 private constant NATIVE_DECIMALS = 18;

    /// @dev erc-7528 native token address convention
    address private constant NATIVE_TOKEN = 0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE;

    ILiquidityManager private immutable LIQUIDITY_MANAGER;
    IStargate private immutable STARGATE;
    address private immutable LZ_ENDPOINT;
    IERC20 private immutable UNDERLYING_TOKEN;
    IzERC20 private immutable ZERC20_TOKEN;
    bool private immutable IS_NATIVE_UNDERLYING;

    // ERC-7201 slot for namespace "zerc20.storage.adaptor".
    bytes32 internal constant ADAPTOR_STORAGE_SLOT = 0x8822ef72de5627cbf701dd2d774295f82a1c725bfbeed7eddf4ec1e237a24400;

    /// @custom:storage-location erc7201:zerc20.storage.adaptor
    struct AdaptorStorage {
        mapping(address => uint256) underlyingTokenBalances;
        mapping(address => uint256) zerc20Balances;
        mapping(address => uint256) nativeBalances;
    }

    function _getAdaptorStorage() private pure returns (AdaptorStorage storage $) {
        bytes32 slot = ADAPTOR_STORAGE_SLOT;
        assembly {
            $.slot := slot
        }
    }

    function liquidityManager() public view returns (ILiquidityManager) {
        return LIQUIDITY_MANAGER;
    }

    function stargate() public view returns (IStargate) {
        return STARGATE;
    }

    function lzEndpoint() public view returns (address) {
        return LZ_ENDPOINT;
    }

    function underlyingToken() public view returns (IERC20) {
        return UNDERLYING_TOKEN;
    }

    function zerc20() public view returns (IzERC20) {
        return ZERC20_TOKEN;
    }

    function underlyingTokenBalances(address user) public view returns (uint256) {
        return _getAdaptorStorage().underlyingTokenBalances[user];
    }

    function zerc20Balances(address user) public view returns (uint256) {
        return _getAdaptorStorage().zerc20Balances[user];
    }

    function nativeBalances(address user) public view returns (uint256) {
        return _getAdaptorStorage().nativeBalances[user];
    }

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

    /// @notice Locks implementation contracts on deployment.
    constructor(address _liquidityManager, address _stargate, address _lzEndpoint) {
        if (_liquidityManager == address(0)) revert ZeroAddress();
        if (_stargate == address(0)) revert ZeroAddress();
        if (_lzEndpoint == address(0)) revert ZeroAddress();
        ILiquidityManager manager = ILiquidityManager(_liquidityManager);
        LIQUIDITY_MANAGER = manager;
        STARGATE = IStargate(_stargate);
        LZ_ENDPOINT = _lzEndpoint;
        UNDERLYING_TOKEN = manager.underlyingToken();
        ZERC20_TOKEN = manager.zerc20();
        IS_NATIVE_UNDERLYING = address(UNDERLYING_TOKEN) == NATIVE_TOKEN;
        if (address(UNDERLYING_TOKEN) == address(0) || address(ZERC20_TOKEN) == address(0)) revert ZeroAddress();
        _disableInitializers();
    }

    /// @notice Initializes the adaptor with its dependencies and owner.
    /// @param _liquidityManager LiquidityManager that wraps/unwraps the zERC20.
    /// @param _stargate Stargate endpoint used for bridging the underlying token.
    /// @param _lzEndpoint LayerZero endpoint that invokes lzCompose.
    /// @param initialOwner Account receiving upgrade authority.
    function initialize(address _liquidityManager, address _stargate, address _lzEndpoint, address initialOwner)
        external
        initializer
    {
        if (_liquidityManager == address(0)) revert ZeroAddress();
        if (_stargate == address(0)) revert ZeroAddress();
        if (_lzEndpoint == address(0)) revert ZeroAddress();
        if (initialOwner == address(0)) revert ZeroAddress();
        if (_liquidityManager != address(LIQUIDITY_MANAGER)) {
            revert LiquidityManagerMismatch(address(LIQUIDITY_MANAGER), _liquidityManager);
        }
        if (_stargate != address(STARGATE)) {
            revert StargateMismatch(address(STARGATE), _stargate);
        }
        if (_lzEndpoint != LZ_ENDPOINT) {
            revert LzEndpointMismatch(LZ_ENDPOINT, _lzEndpoint);
        }

        __Ownable_init();
        __UUPSUpgradeable_init();
        __ReentrancyGuard_init();
        _transferOwnership(initialOwner);

        IERC20 managerUnderlying = LIQUIDITY_MANAGER.underlyingToken();
        IzERC20 managerZerc20 = LIQUIDITY_MANAGER.zerc20();
        if (address(managerUnderlying) != address(UNDERLYING_TOKEN)) {
            revert UnderlyingTokenMismatch(address(UNDERLYING_TOKEN), address(managerUnderlying));
        }
        if (address(managerZerc20) != address(ZERC20_TOKEN)) {
            revert Zerc20Mismatch(address(ZERC20_TOKEN), address(managerZerc20));
        }

        address stargateToken = STARGATE.token();
        if (IS_NATIVE_UNDERLYING) {
            if (stargateToken != NATIVE_TOKEN && stargateToken != address(0)) {
                revert UnderlyingTokenMismatch(address(UNDERLYING_TOKEN), stargateToken);
            }
        } else if (stargateToken != address(UNDERLYING_TOKEN)) {
            revert UnderlyingTokenMismatch(address(UNDERLYING_TOKEN), stargateToken);
        }
    }

    function _authorizeUpgrade(address) internal override onlyOwner {}

    // ---------------------------- External ---------------------------------

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
        AdaptorStorage storage $ = _getAdaptorStorage();
        if (msg.sender != LZ_ENDPOINT) revert InvalidComposeSender();
        if (_from != address(ZERC20_TOKEN)) revert InvalidComposeCaller();

        bytes32 composeFromBytes = OFTComposeMsgCodec.composeFrom(_message);
        address user = OFTComposeMsgCodec.bytes32ToAddress(composeFromBytes);
        uint256 zerc20Amount = OFTComposeMsgCodec.amountLD(_message);

        // record zERC20 & native balance
        $.zerc20Balances[user] += zerc20Amount;
        $.nativeBalances[user] += msg.value;

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

    /// @notice Pulls zERC20 from the caller, unwraps it, and bridges the underlying token per the request.
    /// @param zerc20Amount Amount of zERC20 to unwrap.
    /// @param request Bridge configuration and minimum output expectations.
    function unwrapAndBridge(uint256 zerc20Amount, BridgeRequest calldata request) external payable nonReentrant {
        if (zerc20Amount == 0) revert ZeroAmount();
        _validateBridgeRequest(request);
        address user = msg.sender;
        AdaptorStorage storage $ = _getAdaptorStorage();

        // pull zERC20 from user
        IERC20(address(ZERC20_TOKEN)).safeTransferFrom(user, address(this), zerc20Amount);

        // record zERC20 & native balance
        $.zerc20Balances[user] += zerc20Amount;
        $.nativeBalances[user] += msg.value;

        _unwrapAndBridge(user, zerc20Amount, request);
    }

    /// @notice Withdraws previously deposited tokens from the adaptor.
    function withdraw(address token, uint256 amount) external nonReentrant {
        if (amount == 0) revert ZeroAmount();
        if (token == NATIVE_TOKEN) {
            if (IS_NATIVE_UNDERLYING) {
                _debitCombinedNativeBalance(msg.sender, amount);
            } else {
                _debitNativeBalance(msg.sender, amount);
            }
            (bool success,) = payable(msg.sender).call{value: amount}("");
            if (!success) revert TransferFailed();
        } else if (token == address(UNDERLYING_TOKEN)) {
            if (IS_NATIVE_UNDERLYING) revert InvalidToken();
            _debitUnderlyingBalance(msg.sender, amount);
            UNDERLYING_TOKEN.safeTransfer(msg.sender, amount);
        } else if (token == address(ZERC20_TOKEN)) {
            _debitZerc20Balance(msg.sender, amount);
            IERC20(address(ZERC20_TOKEN)).safeTransfer(msg.sender, amount);
        } else {
            revert InvalidToken();
        }
    }

    // ---------------------------- Views ---------------------------------

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
        uint256 amountAfterDust = _removeStargateDust(amountAfterUnwrap);
        if (amountAfterDust == 0) {
            // Misconfigured Stargate asset or dust rounds to zero; avoid InvalidAmount.
            return FeeQuote({tokenUnwrapFee: tokenUnwrapFee, nativeBridgeFee: 0, tokenBridgeFee: amountAfterUnwrap});
        }

        SendParam memory sendParam = SendParam({
            dstEid: request.dstEid,
            to: request.to.addressToBytes32(),
            amountLD: amountAfterDust,
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
        quote = FeeQuote({
            tokenUnwrapFee: tokenUnwrapFee, nativeBridgeFee: feeQuote.nativeFee, tokenBridgeFee: tokenBridgeFee
        });
    }

    /// @notice Decodes a BridgeRequest from a compose payload.
    /// @param _message Compose payload emitted by the zERC20.
    /// @return request Decoded bridge instructions.
    function decodeBridgeRequest(bytes calldata _message) external pure returns (BridgeRequest memory request) {
        request = abi.decode(OFTComposeMsgCodec.composeMsg(_message), (BridgeRequest));
    }

    // ------------------------- External Self-Calls ----------------------------

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

    // ---------------------- Private functions --------------------

    function _validateBridgeRequest(BridgeRequest calldata request) private pure {
        if (request.to == address(0)) revert ZeroAddress();
        if (request.dstEid == 0) revert InvalidDstEid();
    }

    function _unwrapAndBridge(address user, uint256 zerc20Amount, BridgeRequest memory request) private enableSelfCall {
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

    function _unwrap(address user, uint256 amount, uint256 amountMinOut) private returns (uint256 amountOut) {
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
        _getAdaptorStorage().underlyingTokenBalances[user] += amountOut;
        emit Unwrap(user, amount, amountOut);
    }

    function _bridgeUnderlyingToken(
        address user,
        uint256 amount,
        uint256 nativeBridgeFee,
        BridgeRequest calldata request
    ) private returns (uint256 amountOut) {
        uint256 amountToBridge = _removeStargateDust(amount);
        if (amountToBridge == 0 || amountToBridge < request.minAmountOut) {
            revert OutputTooLow(amountToBridge, request.minAmountOut);
        }

        _debitNativeBalance(user, nativeBridgeFee);
        _debitUnderlyingBalance(user, amountToBridge);

        uint256 actualNativeFee;
        (amountOut, actualNativeFee) = _innerBridgeUnderlyingToken(amountToBridge, nativeBridgeFee, request);
        _applyNativeFeeRefund(user, nativeBridgeFee, actualNativeFee);
        emit BridgeUnderlyingToken(user, request.to, request.dstEid, amountOut, actualNativeFee);
    }

    function _innerBridgeUnderlyingToken(uint256 amount, uint256 nativeBridgeFee, BridgeRequest calldata request)
        private
        returns (uint256 amountOut, uint256 actualNativeFee)
    {
        SendParam memory sendParam = SendParam({
            dstEid: request.dstEid,
            to: request.to.addressToBytes32(),
            amountLD: amount,
            minAmountLD: request.minAmountOut,
            extraOptions: request.extraOptions,
            composeMsg: request.composeMsg,
            oftCmd: request.oftCmd
        });
        _ensureAllowance(UNDERLYING_TOKEN, address(STARGATE), amount);
        MessagingFee memory fee = MessagingFee({nativeFee: nativeBridgeFee, lzTokenFee: 0});

        uint256 sendValue = nativeBridgeFee;
        if (IS_NATIVE_UNDERLYING) {
            sendValue += amount;
        }
        uint256 nativeBalanceBefore = address(this).balance;
        (, OFTReceipt memory oftReceipt,) = STARGATE.sendToken{value: sendValue}(sendParam, fee, address(this));
        amountOut = oftReceipt.amountReceivedLD;
        uint256 nativeBalanceAfter = address(this).balance;
        uint256 totalSpent = nativeBalanceBefore - nativeBalanceAfter;
        actualNativeFee = totalSpent;
        if (IS_NATIVE_UNDERLYING) {
            if (totalSpent < amount) revert AmountMismatch(amount, totalSpent);
            actualNativeFee = totalSpent - amount;
        }
    }

    function _bridgeZerc20(uint32 dstEid, address user, address to, uint256 amount) private {
        _debitZerc20Balance(user, amount);
        bytes memory extraOptions = OptionsBuilder.newOptions().addExecutorLzReceiveOption(RETURN_LZ_RECEIVE_GAS, 0);
        SendParam memory sendParam = SendParam({
            dstEid: dstEid,
            to: to.addressToBytes32(),
            amountLD: amount,
            minAmountLD: 0, // to avoid a revert due to dust removal
            extraOptions: extraOptions,
            composeMsg: bytes(""),
            oftCmd: bytes("")
        });
        MessagingFee memory returnFeeQuote = ZERC20_TOKEN.quoteSend(sendParam, false);
        uint256 nativeFee = returnFeeQuote.nativeFee;

        _debitNativeBalance(user, nativeFee);

        uint256 nativeBalanceBefore = address(this).balance;
        ZERC20_TOKEN.send{value: nativeFee}(sendParam, returnFeeQuote, address(this));
        uint256 nativeBalanceAfter = address(this).balance;
        uint256 actualNativeFee = nativeBalanceBefore - nativeBalanceAfter;
        _applyNativeFeeRefund(user, nativeFee, actualNativeFee);
        emit BridgeZerc20(to, dstEid, amount);
    }

    function _debitNativeBalance(address user, uint256 nativeBridgeFee) private {
        AdaptorStorage storage $ = _getAdaptorStorage();
        uint256 userNativeBalance = $.nativeBalances[user];
        if (userNativeBalance < nativeBridgeFee) revert InsufficientNativeBalance();
        $.nativeBalances[user] = userNativeBalance - nativeBridgeFee;
    }

    function _debitUnderlyingBalance(address user, uint256 amount) private {
        AdaptorStorage storage $ = _getAdaptorStorage();
        uint256 userUnderlyingBalance = $.underlyingTokenBalances[user];
        if (userUnderlyingBalance < amount) revert InsufficientUnderlyingBalance();
        $.underlyingTokenBalances[user] = userUnderlyingBalance - amount;
    }

    function _debitCombinedNativeBalance(address user, uint256 amount) private {
        AdaptorStorage storage $ = _getAdaptorStorage();
        uint256 userUnderlyingBalance = $.underlyingTokenBalances[user];
        uint256 userNativeBalance = $.nativeBalances[user];
        if (userUnderlyingBalance + userNativeBalance < amount) revert InsufficientNativeBalance();
        if (userUnderlyingBalance >= amount) {
            $.underlyingTokenBalances[user] = userUnderlyingBalance - amount;
            return;
        }
        $.underlyingTokenBalances[user] = 0;
        $.nativeBalances[user] = userNativeBalance - (amount - userUnderlyingBalance);
    }

    function _debitZerc20Balance(address user, uint256 amount) private {
        AdaptorStorage storage $ = _getAdaptorStorage();
        uint256 userBalance = $.zerc20Balances[user];
        if (userBalance < amount) revert InsufficientZerc20Balance();
        $.zerc20Balances[user] = userBalance - amount;
    }

    function _ensureAllowance(IERC20 token, address spender, uint256 amount) private {
        if (amount == 0) return;
        if (IS_NATIVE_UNDERLYING) return;
        uint256 currentAllowance = token.allowance(address(this), spender);
        if (currentAllowance >= amount) return;
        token.forceApprove(spender, amount);
    }

    function _applyNativeFeeRefund(address user, uint256 quotedNativeFee, uint256 actualNativeFee) private {
        // Refund surplus native fee if the quote overestimates or refunds are reflected in balance deltas.
        if (quotedNativeFee <= actualNativeFee) return;
        uint256 refundDue = quotedNativeFee - actualNativeFee;
        if (refundDue == 0) return;
        _getAdaptorStorage().nativeBalances[user] += refundDue;
    }

    function _removeStargateDust(uint256 amount) private view returns (uint256 dustlessAmount) {
        uint8 sharedDecimals = STARGATE.sharedDecimals();
        uint8 localDecimals =
            IS_NATIVE_UNDERLYING ? NATIVE_DECIMALS : IERC20Metadata(address(UNDERLYING_TOKEN)).decimals();
        if (localDecimals < sharedDecimals) {
            return 0;
        }
        uint256 conversionRate = 10 ** uint256(localDecimals - sharedDecimals);
        dustlessAmount = amount - (amount % conversionRate);
    }

    function _underlyingBalance() private view returns (uint256) {
        if (IS_NATIVE_UNDERLYING) {
            return address(this).balance;
        }
        return UNDERLYING_TOKEN.balanceOf(address(this));
    }

    /// @notice Accepts native transfers; Stargate/OFT refunds are handled via balance deltas.
    receive() external payable {
        AdaptorStorage storage $ = _getAdaptorStorage();
        if (msg.sender == address(LIQUIDITY_MANAGER) || msg.sender == LZ_ENDPOINT || msg.sender == address(STARGATE)) {
            return;
        }
        $.nativeBalances[msg.sender] += msg.value;
        emit NativeDeposit(msg.sender, msg.value);
    }
}
