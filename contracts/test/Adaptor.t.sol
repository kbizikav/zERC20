// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {TestHelperOz5, EndpointV2} from "@layerzerolabs/test-devtools-evm-foundry/contracts/TestHelperOz5.sol";
import {Adaptor} from "../src/liquidity/Adaptor.sol";
import {ILiquidityManager} from "../src/interfaces/ILiquidityManager.sol";
import {IStargate, Ticket, StargateType} from "../src/interfaces/IStargate.sol";
import {IzERC20} from "../src/interfaces/IzERC20.sol";
import {
    IOFT,
    SendParam,
    MessagingFee,
    MessagingReceipt,
    OFTLimit,
    OFTFeeDetail,
    OFTReceipt
} from "@layerzerolabs/oft-evm/contracts/interfaces/IOFT.sol";
import {OFTComposeMsgCodec} from "@layerzerolabs/oft-evm/contracts/libs/OFTComposeMsgCodec.sol";
import {zERC20} from "../src/zERC20.sol";
import {OFTCoreUpgradeable} from "@layerzerolabs/oft-evm-upgradeable/contracts/oft/OFTCoreUpgradeable.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

contract MintableToken is ERC20 {
    constructor() ERC20("Underlying", "UND") {}

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }
}

contract MockLiquidityManager is ILiquidityManager {
    IERC20 public immutable underlying;
    IzERC20 public immutable zerc20Token;

    uint256 public unwrapFeeQuote;
    bool public unwrapShouldRevert;

    constructor(IERC20 underlying_, address zerc20_) {
        underlying = underlying_;
        zerc20Token = IzERC20(zerc20_);
    }

    function setQuoteUnwrapFee(uint256 fee) external {
        unwrapFeeQuote = fee;
    }

    function setRevertUnwrap(bool shouldRevert) external {
        unwrapShouldRevert = shouldRevert;
    }

    function wrap(uint256, address) external pure override returns (uint256) {
        revert("wrap not implemented");
    }

    function wrapWithMinOut(uint256, uint256, address) external pure override returns (uint256) {
        revert("wrap not implemented");
    }

    function unwrap(uint256 amount, address receiver) external override returns (uint256 amountOut) {
        if (unwrapShouldRevert) revert("unwrap disabled");
        amountOut = amount - unwrapFeeQuote;
        MintableToken(address(underlying)).mint(receiver, amountOut);
    }

    function unwrapWithMinOut(uint256 amount, uint256 minOut, address receiver)
        external
        override
        returns (uint256 amountOut)
    {
        if (unwrapShouldRevert) revert("unwrap disabled");
        amountOut = amount - unwrapFeeQuote;
        if (amountOut < minOut) revert("slippage");
        MintableToken(address(underlying)).mint(receiver, amountOut);
    }

    function quoteWrapReward(uint256) external pure override returns (uint256) {
        return 0;
    }

    function quoteUnwrapFee(uint256) external view override returns (uint256) {
        return unwrapFeeQuote;
    }

    function underlyingToken() external view override returns (IERC20) {
        return underlying;
    }

    function zerc20() external view override returns (IzERC20) {
        return zerc20Token;
    }

    function feeSurplus() external pure override returns (uint256) {
        return 0;
    }

    function withdrawRewards(address, uint256) external pure override {
        revert("withdrawRewards disabled");
    }
}

contract MockStargate is IStargate {
    IERC20 public immutable underlying;

    uint256 public nativeFeeQuote;
    uint256 public tokenFee;
    uint256 public bonus;
    SendParam public lastSendParam;
    uint256 public lastValue;
    address public lastRefund;
    bool public revertSend;

    constructor(IERC20 underlying_) {
        underlying = underlying_;
    }

    function setQuote(uint256 nativeFee, uint256 tokenFee_) external {
        nativeFeeQuote = nativeFee;
        tokenFee = tokenFee_;
    }

    function setBonus(uint256 bonus_) external {
        bonus = bonus_;
    }

    function setRevertSend(bool shouldRevert) external {
        revertSend = shouldRevert;
    }

    function lastSendParamAmount() external view returns (uint256) {
        return lastSendParam.amountLD;
    }

    function oftVersion() external pure override returns (bytes4 interfaceId, uint64 version) {
        interfaceId = 0x02e49c2c;
        version = 1;
    }

    function token() external view override returns (address) {
        return address(underlying);
    }

    function approvalRequired() external pure override returns (bool) {
        return true;
    }

    function sharedDecimals() external pure override returns (uint8) {
        return 18;
    }

    function quoteOFT(
        SendParam calldata _sendParam
    ) external view override returns (OFTLimit memory limit, OFTFeeDetail[] memory oftFeeDetails, OFTReceipt memory receipt) {
        limit = OFTLimit({minAmountLD: 0, maxAmountLD: type(uint256).max});
        oftFeeDetails = new OFTFeeDetail[](0);
        uint256 amountReceived = _sendParam.amountLD > tokenFee ? _sendParam.amountLD - tokenFee : 0;
        if (bonus > 0) {
            amountReceived += bonus;
        }
        receipt = OFTReceipt({amountSentLD: _sendParam.amountLD, amountReceivedLD: amountReceived});
    }

    function quoteSend(SendParam calldata, bool) external view override returns (MessagingFee memory) {
        return MessagingFee({nativeFee: nativeFeeQuote, lzTokenFee: 0});
    }

    function sendToken(
        SendParam calldata _sendParam,
        MessagingFee calldata _fee,
        address _refundAddress
    )
        external
        payable
        override
        returns (MessagingReceipt memory msgReceipt, OFTReceipt memory oftReceipt, Ticket memory)
    {
        if (revertSend) revert("sendToken reverted");
        lastSendParam = _sendParam;
        lastValue = msg.value;
        lastRefund = _refundAddress;

        require(underlying.transferFrom(msg.sender, address(this), _sendParam.amountLD), "transfer failed");

        msgReceipt = MessagingReceipt({
            guid: bytes32(0),
            nonce: 0,
            fee: MessagingFee({nativeFee: msg.value, lzTokenFee: _fee.lzTokenFee})
        });
        uint256 amountReceived = _sendParam.amountLD > tokenFee ? _sendParam.amountLD - tokenFee : 0;
        if (bonus > 0) {
            amountReceived += bonus;
        }
        oftReceipt = OFTReceipt({amountSentLD: _sendParam.amountLD, amountReceivedLD: amountReceived});
    }

    function send(
        SendParam calldata _sendParam,
        MessagingFee calldata _fee,
        address _refundAddress
    ) external payable override returns (MessagingReceipt memory msgReceipt, OFTReceipt memory oftReceipt) {
        if (revertSend) revert("sendToken reverted");
        lastSendParam = _sendParam;
        lastValue = msg.value;
        lastRefund = _refundAddress;

        require(underlying.transferFrom(msg.sender, address(this), _sendParam.amountLD), "transfer failed");

        msgReceipt = MessagingReceipt({
            guid: bytes32(0),
            nonce: 0,
            fee: MessagingFee({nativeFee: msg.value, lzTokenFee: _fee.lzTokenFee})
        });
        uint256 amountReceived = _sendParam.amountLD > tokenFee ? _sendParam.amountLD - tokenFee : 0;
        if (bonus > 0) {
            amountReceived += bonus;
        }
        oftReceipt = OFTReceipt({amountSentLD: _sendParam.amountLD, amountReceivedLD: amountReceived});
    }

    function stargateType() external pure override returns (StargateType) {
        return StargateType.OFT;
    }
}

contract ZERC20AdaptorHarness is zERC20 {
    uint256 public quoteNativeFee;
    uint256 public quoteLzFee;
    uint256 public lastSendValue;
    SendParam public lastSendParam;

    constructor(address endpoint) zERC20(endpoint, 18) {}

    function setQuoteSendFee(uint256 nativeFee) external {
        quoteNativeFee = nativeFee;
    }

    function lastSendParamAmount() external view returns (uint256) {
        return lastSendParam.amountLD;
    }

    function quoteSend(SendParam calldata, bool) public view override(IOFT, OFTCoreUpgradeable) returns (MessagingFee memory) {
        return MessagingFee({nativeFee: quoteNativeFee, lzTokenFee: quoteLzFee});
    }

    function send(
        SendParam calldata _sendParam,
        MessagingFee calldata _fee,
        address
    ) public payable override(IOFT, OFTCoreUpgradeable) returns (MessagingReceipt memory msgReceipt, OFTReceipt memory oftReceipt) {
        if (msg.value != _fee.nativeFee) revert("native fee mismatch");
        lastSendParam = _sendParam;
        lastSendValue = msg.value;

        (uint256 amountSentLD, uint256 amountReceivedLD) =
            _debit(msg.sender, _sendParam.amountLD, _sendParam.minAmountLD, _sendParam.dstEid);

        msgReceipt = MessagingReceipt({
            guid: bytes32(0),
            nonce: 0,
            fee: MessagingFee({nativeFee: msg.value, lzTokenFee: _fee.lzTokenFee})
        });
        oftReceipt = OFTReceipt({amountSentLD: amountSentLD, amountReceivedLD: amountReceivedLD});
    }
}

contract AdaptorTest is TestHelperOz5 {
    Adaptor internal adaptor;
    MockLiquidityManager internal manager;
    MockStargate internal stargate;
    ZERC20AdaptorHarness internal zerc20;
    MintableToken internal underlying;

    EndpointV2 internal endpoint;

    address internal constant USER = address(0xA11CE);
    address internal constant DESTINATION = address(0xB0B);
    address internal constant NATIVE_TOKEN = 0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE;
    uint32 internal constant DST_EID = 101;

    function setUp() public override {
        super.setUp();
        endpoint = new EndpointV2(1, address(this));

        underlying = new MintableToken();
        zerc20 = _deployZerc20(endpoint);
        manager = new MockLiquidityManager(underlying, address(zerc20));
        stargate = new MockStargate(underlying);
        adaptor = new Adaptor(address(manager), address(stargate), address(endpoint));

        zerc20.setMinter(address(this));
    }

    function testConstructorRevertsOnStargateTokenMismatch() public {
        MintableToken otherUnderlying = new MintableToken();
        MockStargate badStargate = new MockStargate(otherUnderlying);

        vm.expectRevert(
            abi.encodeWithSelector(
                Adaptor.UnderlyingTokenMismatch.selector, address(underlying), address(otherUnderlying)
            )
        );
        new Adaptor(address(manager), address(badStargate), address(endpoint));
    }

    function testQuoteFeeSaturatesBridgeFee() public {
        uint256 amount = 10 ether;

        manager.setQuoteUnwrapFee(0);
        stargate.setQuote(0, 0);
        stargate.setBonus(1 ether);

        Adaptor.BridgeRequest memory request = Adaptor.BridgeRequest({
            dstEid: DST_EID,
            to: DESTINATION,
            minAmountOut: 0,
            extraOptions: bytes(""),
            composeMsg: bytes(""),
            oftCmd: bytes("")
        });

        Adaptor.FeeQuote memory quote = adaptor.quoteFee(amount, request);
        assertEq(quote.tokenBridgeFee, 0, "bridge fee floors at 0");
    }

    function testReceiveCreditsNativeBalance() public {
        uint256 amount = 0.15 ether;

        vm.deal(USER, amount);
        vm.prank(USER);
        (bool success,) = address(adaptor).call{value: amount}("");
        assertTrue(success, "receive failed");

        assertEq(adaptor.nativeBalances(USER), amount, "native credited");

        vm.prank(USER);
        adaptor.withdraw(NATIVE_TOKEN, amount);

        assertEq(adaptor.nativeBalances(USER), 0, "native balance cleared");
        assertEq(address(USER).balance, amount, "native returned");
    }

    function testUnwrapAndBridgeConsumesFeesAndEmits() public {
        uint256 amount = 100 ether;
        uint256 unwrapFee = 1 ether;
        uint256 bridgeTokenFee = 0.5 ether;
        uint256 nativeFee = 0.05 ether;

        manager.setQuoteUnwrapFee(unwrapFee);
        stargate.setQuote(nativeFee, bridgeTokenFee);

        zerc20.mint(USER, amount);
        vm.prank(USER);
        zerc20.approve(address(adaptor), amount);

        vm.deal(USER, nativeFee);

        Adaptor.BridgeRequest memory request = Adaptor.BridgeRequest({
            dstEid: DST_EID,
            to: DESTINATION,
            minAmountOut: amount - unwrapFee - bridgeTokenFee - 1,
            extraOptions: bytes(""),
            composeMsg: bytes(""),
            oftCmd: bytes("")
        });

        uint256 expectedUnderlying = amount - unwrapFee;
        uint256 expectedBridged = expectedUnderlying - bridgeTokenFee;

        vm.expectEmit(true, true, false, true, address(adaptor));
        emit Adaptor.Unwrap(USER, amount, expectedUnderlying);
        vm.expectEmit(true, true, true, true, address(adaptor));
        emit Adaptor.BridgeUnderlyingToken(USER, DESTINATION, DST_EID, expectedBridged, nativeFee);
        vm.expectEmit(true, true, false, true, address(adaptor));
        emit Adaptor.UnwrapAndBridge(USER, amount, expectedBridged, DESTINATION, DST_EID);

        vm.prank(USER);
        adaptor.unwrapAndBridge{value: nativeFee}(amount, request);

        assertEq(adaptor.zerc20Balances(USER), 0, "zerc20 balance cleared");
        assertEq(adaptor.underlingTokenBalances(USER), 0, "underlying balance cleared");
        assertEq(adaptor.nativeBalances(USER), 0, "native balance cleared");
        assertEq(underlying.balanceOf(address(adaptor)), 0, "underlying forwarded");
        assertEq(underlying.balanceOf(address(stargate)), expectedUnderlying, "stargate received underlying");
        assertEq(underlying.allowance(address(adaptor), address(stargate)), 0, "allowance reset");
        assertEq(stargate.lastSendParamAmount(), expectedUnderlying, "bridged amount");
        assertEq(stargate.lastValue(), nativeFee, "native fee forwarded");
    }

    function testUnwrapAndBridgeReturnsZerc20OnLowOutput() public {
        uint256 amount = 40 ether;
        uint256 returnNativeFee = 0.02 ether;

        manager.setQuoteUnwrapFee(0);
        stargate.setQuote(returnNativeFee, 0);
        zerc20.setQuoteSendFee(returnNativeFee);
        manager.setRevertUnwrap(true);

        zerc20.mint(USER, amount);
        vm.prank(USER);
        zerc20.approve(address(adaptor), amount);

        vm.deal(USER, returnNativeFee);

        Adaptor.BridgeRequest memory request = Adaptor.BridgeRequest({
            dstEid: DST_EID,
            to: DESTINATION,
            minAmountOut: amount + 1, // force slippage fail
            extraOptions: bytes(""),
            composeMsg: bytes(""),
            oftCmd: bytes("")
        });

        vm.expectEmit(true, true, true, true, address(adaptor));
        emit Adaptor.BridgeZerc20(DESTINATION, DST_EID, amount);

        vm.prank(USER);
        adaptor.unwrapAndBridge{value: returnNativeFee}(amount, request);

        assertEq(adaptor.zerc20Balances(USER), 0, "zerc20 balance cleared");
        assertEq(adaptor.underlingTokenBalances(USER), 0, "no underlying credited");
        assertEq(adaptor.nativeBalances(USER), 0, "native balance cleared");
        assertEq(zerc20.balanceOf(address(adaptor)), 0, "zerc20 bridged back");
        assertEq(zerc20.lastSendParamAmount(), amount, "zerc20 bridged amount");
        assertEq(zerc20.lastSendValue(), returnNativeFee, "native fee used for return");
    }

    function testLzComposeRejectsNonEndpointCaller() public {
        bytes memory message = bytes("invalid");

        vm.expectRevert(Adaptor.InvalidComposeSender.selector);
        adaptor.lzCompose(address(zerc20), bytes32(0), message, address(0), bytes(""));
    }

    function testLzComposeEmitsDecodeFailureAndAllowsWithdraw() public {
        uint256 amount = 25 ether;
        uint256 nativeFee = 0.01 ether;
        zerc20.mint(address(adaptor), amount);

        bytes memory composeMsg = abi.encodePacked(OFTComposeMsgCodec.addressToBytes32(USER));
        bytes memory message = OFTComposeMsgCodec.encode(0, DST_EID, amount, composeMsg);

        vm.expectEmit(false, false, false, false, address(adaptor));
        emit Adaptor.DecodeBridgeRequestFailed(message, bytes("")); // revertData intentionally unchecked

        vm.deal(address(endpoint), nativeFee);
        vm.prank(address(endpoint));
        adaptor.lzCompose{value: nativeFee}(address(zerc20), bytes32(0), message, address(0), bytes(""));

        assertEq(adaptor.zerc20Balances(USER), amount, "zerc20 credited");
        assertEq(adaptor.nativeBalances(USER), nativeFee, "native credited");

        vm.prank(USER);
        adaptor.withdraw(address(zerc20), amount);
        vm.prank(USER);
        adaptor.withdraw(NATIVE_TOKEN, nativeFee);

        assertEq(adaptor.zerc20Balances(USER), 0, "zerc20 balance cleared after withdraw");
        assertEq(adaptor.nativeBalances(USER), 0, "native balance cleared after withdraw");
        assertEq(zerc20.balanceOf(USER), amount, "zerc20 returned");
        assertEq(address(USER).balance, nativeFee, "native returned");
    }

    function testLzComposeBridgeFailureLeavesBalancesWithdrawable() public {
        uint256 amount = 80 ether;
        uint256 nativeFee = 0.05 ether;

        manager.setQuoteUnwrapFee(0);
        stargate.setQuote(nativeFee, 0);
        stargate.setRevertSend(true);
        zerc20.mint(address(adaptor), amount);

        Adaptor.BridgeRequest memory request = Adaptor.BridgeRequest({
            dstEid: DST_EID,
            to: DESTINATION,
            minAmountOut: amount,
            extraOptions: bytes(""),
            composeMsg: bytes(""),
            oftCmd: bytes("")
        });
        bytes memory message = _buildComposeMessage(USER, amount, request);

        vm.expectEmit(true, true, false, true, address(adaptor));
        emit Adaptor.Unwrap(USER, amount, amount);

        vm.deal(address(endpoint), nativeFee);
        vm.prank(address(endpoint));
        adaptor.lzCompose{value: nativeFee}(address(zerc20), bytes32(0), message, address(0), bytes(""));

        assertEq(adaptor.zerc20Balances(USER), 0, "zerc20 debited during unwrap");
        assertEq(adaptor.underlingTokenBalances(USER), amount, "underlying credited");
        assertEq(adaptor.nativeBalances(USER), nativeFee, "native still held after bridge revert");
        assertEq(underlying.balanceOf(address(adaptor)), amount, "underlying retained for withdraw");

        vm.prank(USER);
        adaptor.withdraw(address(underlying), amount);
        vm.prank(USER);
        adaptor.withdraw(NATIVE_TOKEN, nativeFee);

        assertEq(underlying.balanceOf(USER), amount, "underlying withdrawable");
        assertEq(address(USER).balance, nativeFee, "native withdrawable");
    }

    function testLzComposeReturnsZerc20OnLowOutput() public {
        uint256 amount = 40 ether;
        uint256 returnNativeFee = 0.02 ether;

        manager.setQuoteUnwrapFee(0);
        stargate.setQuote(returnNativeFee, 0);
        zerc20.setQuoteSendFee(returnNativeFee);
        zerc20.mint(address(adaptor), amount);

        Adaptor.BridgeRequest memory request = Adaptor.BridgeRequest({
            dstEid: DST_EID,
            to: DESTINATION,
            minAmountOut: amount + 1, // force slippage fail inside lzCompose
            extraOptions: bytes(""),
            composeMsg: bytes(""),
            oftCmd: bytes("")
        });
        bytes memory message = _buildComposeMessage(USER, amount, request);

        vm.expectEmit(true, true, true, true, address(adaptor));
        emit Adaptor.BridgeZerc20(DESTINATION, DST_EID, amount);

        vm.deal(address(endpoint), returnNativeFee);
        vm.prank(address(endpoint));
        adaptor.lzCompose{value: returnNativeFee}(address(zerc20), bytes32(0), message, address(0), bytes(""));

        assertEq(adaptor.zerc20Balances(USER), 0, "zerc20 debited for return");
        assertEq(adaptor.underlingTokenBalances(USER), 0, "no underlying credited");
        assertEq(adaptor.nativeBalances(USER), 0, "native spent for return send");
        assertEq(zerc20.balanceOf(address(adaptor)), 0, "zerc20 sent back");
        assertEq(zerc20.lastSendParamAmount(), amount, "returned amount");
        assertEq(zerc20.lastSendValue(), returnNativeFee, "native fee consumed");
    }

    function _deployZerc20(EndpointV2 endpointMock) private returns (ZERC20AdaptorHarness) {
        ZERC20AdaptorHarness impl = new ZERC20AdaptorHarness(address(endpointMock));
        bytes memory initData = abi.encodeCall(zERC20.initialize, ("Zero Token", "ZTK", address(this)));
        ERC1967Proxy proxy = new ERC1967Proxy(address(impl), initData);
        return ZERC20AdaptorHarness(payable(address(proxy)));
    }

    function _buildComposeMessage(
        address user,
        uint256 amount,
        Adaptor.BridgeRequest memory request
    ) private pure returns (bytes memory) {
        bytes memory composeMsg = abi.encodePacked(OFTComposeMsgCodec.addressToBytes32(user), abi.encode(request));
        return OFTComposeMsgCodec.encode(0, DST_EID, amount, composeMsg);
    }
}
