// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {LiquidityManager} from "../src/liquidity/LiquidityManager.sol";
import {IncentiveLib} from "../src/libraries/IncentiveLib.sol";
import {zERC20} from "../src/zERC20.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {EndpointV2Mock as EndpointV2} from "@layerzerolabs/test-devtools-evm-foundry/contracts/mocks/EndpointV2Mock.sol";

contract MintableERC20 is ERC20 {
    uint8 private immutable _decimals;

    constructor(string memory name_, string memory symbol_, uint8 decimals_) ERC20(name_, symbol_) {
        _decimals = decimals_;
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function decimals() public view override returns (uint8) {
        return _decimals;
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

    function setUp() public {
        endpoint = new EndpointV2(1, address(this));
        token = _deployToken(address(this), endpoint, 18);
        underlying = new MintableERC20("Underlying", "UND", 18);

        params = IncentiveLib.FeeParams({targetLiquidity: 1_000 ether, k: 1_000});
        manager = _deployManager(address(underlying), address(token), params, address(this));
        token.setMinter(address(manager));

        underlying.mint(ALICE, START_BALANCE);
    }

    function testInitializeRevertsOnDecimalMismatch() public {
        MintableERC20 usdc = new MintableERC20("USDC", "USDC", 6);
        LiquidityManager impl = new LiquidityManager();
        bytes memory initData =
            abi.encodeCall(LiquidityManager.initialize, (address(usdc), address(token), params, address(this)));

        vm.expectRevert(LiquidityManager.DecimalMismatch.selector);
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

        vm.expectRevert(IncentiveLib.InvalidTarget.selector);
        manager.setFeeParams(IncentiveLib.FeeParams({targetLiquidity: 0, k: 1}));

        vm.expectRevert(IncentiveLib.InvalidK.selector);
        manager.setFeeParams(IncentiveLib.FeeParams({targetLiquidity: 1, k: 10_001}));
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
        LiquidityManager implementation = new LiquidityManager();
        bytes memory initData = abi.encodeCall(LiquidityManager.initialize, (underlying_, zerc20_, feeParams, owner));
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        return LiquidityManager(address(proxy));
    }

    function _deployToken(address owner, EndpointV2 endpointMock, uint8 decimals_) private returns (zERC20) {
        zERC20 impl = new zERC20(address(endpointMock), decimals_);
        bytes memory initData = abi.encodeCall(zERC20.initialize, ("Zero Token", "ZTK", owner));
        ERC1967Proxy proxy = new ERC1967Proxy(address(impl), initData);
        return zERC20(address(proxy));
    }
}
