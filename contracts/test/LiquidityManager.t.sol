// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {LiquidityManager} from "../src/liquidity/LiquidityManager.sol";
import {IncentiveLib} from "../src/libraries/IncentiveLib.sol";
import {zERC20} from "../src/zERC20.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {
    EndpointV2Mock as EndpointV2
} from "@layerzerolabs/test-devtools-evm-foundry/contracts/mocks/EndpointV2Mock.sol";

contract MintableERC20 is ERC20 {
    uint8 private immutable DECIMALS;

    constructor(string memory name_, string memory symbol_, uint8 decimals_) ERC20(name_, symbol_) {
        DECIMALS = decimals_;
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function decimals() public view override returns (uint8) {
        return DECIMALS;
    }
}

contract LiquidityManagerTest is Test {
    LiquidityManager internal manager;
    zERC20 internal token;
    MintableERC20 internal underlying;
    EndpointV2 internal endpoint;
    IncentiveLib.FeeParams internal params;

    address internal constant ALICE = address(0xA11CE);
    address internal constant REWARD_COLLECTOR = address(0xC0FFEE);
    uint256 internal constant START_BALANCE = 10_000 ether;
    address internal constant NATIVE_TOKEN = 0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE;

    function setUp() public {
        endpoint = new EndpointV2(1, address(this));
        token = _deployToken(address(this), endpoint, 18);
        underlying = new MintableERC20("Underlying", "UND", 18);

        params = IncentiveLib.FeeParams({targetLiquidity: 1_000 ether, k: 1_000});
        manager = _deployManager(address(underlying), address(token), params, address(this));
        token.setMinter(address(manager));

        underlying.mint(ALICE, START_BALANCE);
    }

    function testConstructorRevertsOnZeroUnderlyingToken() public {
        vm.expectRevert(LiquidityManager.ZeroAddress.selector);
        new LiquidityManager(address(0), address(token));
    }

    function testConstructorRevertsOnZeroZerc20() public {
        vm.expectRevert(LiquidityManager.ZeroAddress.selector);
        new LiquidityManager(address(underlying), address(0));
    }

    function testInitializeRevertsOnDecimalMismatch() public {
        MintableERC20 usdc = new MintableERC20("USDC", "USDC", 6);
        LiquidityManager impl = new LiquidityManager(address(usdc), address(token));
        bytes memory initData = abi.encodeCall(LiquidityManager.initialize, (params, address(this)));

        vm.expectRevert(LiquidityManager.DecimalMismatch.selector);
        new ERC1967Proxy(address(impl), initData);
    }

    function testInitializeRevertsOnZeroOwner() public {
        LiquidityManager impl = new LiquidityManager(address(underlying), address(token));
        bytes memory initData = abi.encodeCall(LiquidityManager.initialize, (params, address(0)));

        vm.expectRevert(LiquidityManager.ZeroAddress.selector);
        new ERC1967Proxy(address(impl), initData);
    }

    function testWrapThenUnwrapAccruesFeeAndPaysRewards() public {
        vm.prank(ALICE);
        underlying.approve(address(manager), type(uint256).max);

        uint256 firstQuote = manager.quoteWrapReward(500 ether);
        assertEq(firstQuote, 0, "no rewards without surplus");

        vm.prank(ALICE);
        uint256 minted = manager.wrap(500 ether, ALICE);

        uint256 fee = manager.quoteUnwrapFee(200 ether);

        vm.prank(ALICE);
        uint256 received = manager.unwrap(200 ether, ALICE);

        assertEq(received, 200 ether - fee, "unwrap amount");
        assertEq(manager.feeSurplus(), fee, "fee surplus grows");
        assertEq(minted, 500 ether, "wrap returns principal when surplus empty");
        assertEq(token.balanceOf(ALICE), minted - 200 ether, "zerc20 after burn");

        uint256 reward = manager.quoteWrapReward(100 ether);
        uint256 surplusBefore = manager.feeSurplus();

        vm.prank(ALICE);
        uint256 mintedWithReward = manager.wrap(100 ether, ALICE);

        assertEq(mintedWithReward, 100 ether + reward, "reward added");
        assertEq(manager.feeSurplus(), surplusBefore - reward, "surplus consumed");
        assertEq(token.balanceOf(ALICE), minted - 200 ether + mintedWithReward, "zerc20 balance accumulates");

        uint256 expectedUnderlying = 400 ether + fee;
        assertEq(underlying.balanceOf(address(manager)), expectedUnderlying, "underlying held by manager");
    }

    function testWrapWithMinOutRevertsOnSlippage() public {
        _accrueFeeSurplus();

        uint256 reward = manager.quoteWrapReward(100 ether);
        uint256 minOut = 100 ether + reward + 1;

        vm.prank(ALICE);
        vm.expectRevert(LiquidityManager.SlippageExceeded.selector);
        manager.wrapWithMinOut(100 ether, minOut, ALICE);
    }

    function testWrapWithMinOutSucceeds() public {
        _accrueFeeSurplus();

        uint256 reward = manager.quoteWrapReward(100 ether);
        uint256 minOut = 100 ether + reward;

        vm.prank(ALICE);
        uint256 minted = manager.wrapWithMinOut(100 ether, minOut, ALICE);

        assertEq(minted, minOut, "wrap with min out");
    }

    function testUnwrapWithMinOutRevertsOnSlippage() public {
        vm.startPrank(ALICE);
        underlying.approve(address(manager), type(uint256).max);
        manager.wrap(500 ether, ALICE);

        uint256 fee = manager.quoteUnwrapFee(200 ether);
        uint256 minOut = 200 ether - fee + 1;

        vm.expectRevert(LiquidityManager.SlippageExceeded.selector);
        manager.unwrapWithMinOut(200 ether, minOut, ALICE);
        vm.stopPrank();
    }

    function testUnwrapWithMinOutSucceeds() public {
        vm.startPrank(ALICE);
        underlying.approve(address(manager), type(uint256).max);
        manager.wrap(500 ether, ALICE);

        uint256 fee = manager.quoteUnwrapFee(200 ether);
        uint256 minOut = 200 ether - fee;
        uint256 received = manager.unwrapWithMinOut(200 ether, minOut, ALICE);

        vm.stopPrank();
        assertEq(received, minOut, "unwrap with min out");
    }

    function testWrapAndUnwrapNativeUnderlying() public {
        zERC20 nativeToken = _deployToken(address(this), endpoint, 18);
        LiquidityManager nativeManager = _deployManager(NATIVE_TOKEN, address(nativeToken), params, address(this));
        nativeToken.setMinter(address(nativeManager));

        uint256 amount = 2 ether;
        vm.deal(ALICE, amount);

        vm.prank(ALICE);
        uint256 minted = nativeManager.wrap{value: amount}(amount, ALICE);

        assertEq(minted, amount, "native wrap mints principal");
        assertEq(nativeToken.balanceOf(ALICE), amount, "zerc20 minted to receiver");
        assertEq(address(nativeManager).balance, amount, "native held by manager");

        uint256 fee = nativeManager.quoteUnwrapFee(amount);

        vm.prank(ALICE);
        uint256 received = nativeManager.unwrap(amount, ALICE);

        assertEq(received, amount - fee, "unwrap pays fee");
        assertEq(nativeManager.feeSurplus(), fee, "fee surplus grows");
        assertEq(address(ALICE).balance, amount - fee, "native sent to receiver");
        assertEq(address(nativeManager).balance, fee, "native fee retained");
    }

    function testWrapNativeRevertsOnValueMismatch() public {
        LiquidityManager nativeManager = _deployManager(NATIVE_TOKEN, address(token), params, address(this));
        token.setMinter(address(nativeManager));

        vm.deal(ALICE, 1 ether);
        vm.prank(ALICE);
        vm.expectRevert(abi.encodeWithSelector(LiquidityManager.InvalidMsgValue.selector, 2 ether, 1 ether));
        nativeManager.wrap{value: 1 ether}(2 ether, ALICE);
    }

    function testWithdrawRewardsRequiresAdminAndEmitsSurplus() public {
        uint256 accruedFee = _accrueFeeSurplus();

        vm.prank(ALICE);
        vm.expectRevert();
        manager.withdrawRewards(REWARD_COLLECTOR, accruedFee);

        vm.expectRevert(LiquidityManager.InsufficientRewards.selector);
        manager.withdrawRewards(REWARD_COLLECTOR, accruedFee + 1);

        uint256 receiverBefore = underlying.balanceOf(REWARD_COLLECTOR);
        manager.withdrawRewards(REWARD_COLLECTOR, accruedFee);

        assertEq(underlying.balanceOf(REWARD_COLLECTOR), receiverBefore + accruedFee, "rewards transferred");
        assertEq(manager.feeSurplus(), 0, "surplus cleared");
    }

    function testSetFeeParamsRestrictedAndValidatesTarget() public {
        IncentiveLib.FeeParams memory newParams = IncentiveLib.FeeParams({targetLiquidity: 2_000 ether, k: 2_000});

        vm.prank(ALICE);
        vm.expectRevert();
        manager.setFeeParams(newParams);

        manager.setFeeParams(newParams);
        IncentiveLib.FeeParams memory stored = manager.feeParams();
        assertEq(stored.targetLiquidity, newParams.targetLiquidity, "target stored");
        assertEq(stored.k, newParams.k, "k stored");

        vm.expectRevert(IncentiveLib.InvalidK.selector);
        manager.setFeeParams(IncentiveLib.FeeParams({targetLiquidity: 1, k: 10_001}));
    }

    function testZeroTargetLiquidityDisablesFeesAndRewards() public {
        uint256 surplus = _accrueFeeSurplus();

        manager.setFeeParams(IncentiveLib.FeeParams({targetLiquidity: 0, k: 1_000}));

        assertEq(manager.quoteWrapReward(100 ether), 0, "wrap reward quote zero");
        assertEq(manager.quoteUnwrapFee(100 ether), 0, "unwrap fee quote zero");

        vm.startPrank(ALICE);
        underlying.approve(address(manager), type(uint256).max);

        uint256 minted = manager.wrap(100 ether, ALICE);
        assertEq(minted, 100 ether, "wrap mints only principal");
        assertEq(manager.feeSurplus(), surplus, "surplus unchanged after wrap");

        uint256 received = manager.unwrap(50 ether, ALICE);
        assertEq(received, 50 ether, "unwrap returns only principal");
        assertEq(manager.feeSurplus(), surplus, "surplus unchanged after unwrap");
        vm.stopPrank();
    }

    function testSetFeeParamsRejectsOversizedTarget() public {
        uint256 tooLargeTarget = uint256(type(uint128).max) + 1;
        IncentiveLib.FeeParams memory newParams = IncentiveLib.FeeParams({targetLiquidity: tooLargeTarget, k: 1});

        vm.expectRevert(IncentiveLib.InvalidTarget.selector);
        manager.setFeeParams(newParams);
    }

    function testSetFeeParamsRejectsOverflowingK() public {
        uint256 largeTarget = type(uint128).max;
        IncentiveLib.FeeParams memory newParams = IncentiveLib.FeeParams({targetLiquidity: largeTarget, k: 2});

        vm.expectRevert(IncentiveLib.InvalidK.selector);
        manager.setFeeParams(newParams);
    }

    function _accrueFeeSurplus() private returns (uint256 feeAmount) {
        vm.startPrank(ALICE);
        underlying.approve(address(manager), type(uint256).max);

        manager.wrap(400 ether, ALICE);
        feeAmount = manager.quoteUnwrapFee(150 ether);
        manager.unwrap(150 ether, ALICE);

        vm.stopPrank();

        assertGt(feeAmount, 0, "fee should accumulate");
        return feeAmount;
    }

    function _deployManager(
        address underlying_,
        address zerc20_,
        IncentiveLib.FeeParams memory feeParams,
        address owner
    ) private returns (LiquidityManager) {
        LiquidityManager implementation = new LiquidityManager(underlying_, zerc20_);
        bytes memory initData = abi.encodeCall(LiquidityManager.initialize, (feeParams, owner));
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        return LiquidityManager(payable(address(proxy)));
    }

    function _deployToken(address owner, EndpointV2 endpointMock, uint8 decimals_) private returns (zERC20) {
        zERC20 impl = new zERC20(address(endpointMock), decimals_);
        bytes memory initData = abi.encodeCall(zERC20.initialize, ("Zero Token", "ZTK", owner));
        ERC1967Proxy proxy = new ERC1967Proxy(address(impl), initData);
        return zERC20(address(proxy));
    }
}
