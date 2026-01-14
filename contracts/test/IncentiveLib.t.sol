// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Test} from "forge-std/Test.sol";
import {IncentiveLib} from "../src/libraries/IncentiveLib.sol";

contract IncentiveLibHarness {
    function quoteWrapReward(
        IncentiveLib.FeeParams calldata params,
        uint256 liquidity,
        uint256 feeSurplus,
        uint256 amount
    ) external pure returns (uint256) {
        return IncentiveLib.quoteWrapReward(params, liquidity, feeSurplus, amount);
    }

    function quoteUnwrapFee(IncentiveLib.FeeParams calldata params, uint256 liquidity, uint256 amount)
        external
        pure
        returns (uint256)
    {
        return IncentiveLib.quoteUnwrapFee(params, liquidity, amount);
    }
}

contract IncentiveLibTest is Test {
    IncentiveLibHarness internal lib;
    uint256 internal constant TEN_PERCENT_K = 1_000; // 0.1 in basis points terms

    function setUp() public {
        lib = new IncentiveLibHarness();
    }

    function testWrapRewardRespectsFeeSurplusCap() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 reward = lib.quoteWrapReward(params, 0, 40, 1_000);

        assertEq(reward, 40, "wrap reward should cap at fee surplus");
    }

    function testWrapRewardFloorsFractionalResult() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 reward = lib.quoteWrapReward(params, 400, type(uint256).max, 300);

        assertEq(reward, 13, "wrap reward should floor fractional area");
    }

    function testWrapFromZeroUpToHalfTarget() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 reward = lib.quoteWrapReward(params, 0, type(uint256).max, 500);

        assertEq(reward, 37, "wrap reward from 0 to mid target");
    }

    function testWrapFromZeroToTarget() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 reward = lib.quoteWrapReward(params, 0, type(uint256).max, 1_000);

        assertEq(reward, 50, "wrap reward from 0 to target");
    }

    function testWrapCrossesTargetOnlyPaysBelowTarget() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 reward = lib.quoteWrapReward(params, 800, type(uint256).max, 400);

        assertEq(reward, 2, "only below-target segment should earn reward");
    }

    function testWrapWhenAlreadyAboveTargetHasNoReward() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 reward = lib.quoteWrapReward(params, 1_200, type(uint256).max, 100);

        assertEq(reward, 0, "no reward when always above target");
    }

    function testUnwrapFeeCeilsFractionalResult() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 fee = lib.quoteUnwrapFee(params, 700, 250);

        assertEq(fee, 11, "unwrap fee should ceil fractional area");
    }

    function testUnwrapFeeRoundsUpTinyAmounts() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 fee = lib.quoteUnwrapFee(params, 100, 1);

        assertEq(fee, 1, "tiny fractional fee should round up to 1");
    }

    function testUnwrapFromTargetToZero() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 fee = lib.quoteUnwrapFee(params, 1_000, 1_000);

        assertEq(fee, 50, "unwrap fee mirrors wrap from target to zero");
    }

    function testUnwrapAboveTargetStaysAboveTargetHasNoFee() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 fee = lib.quoteUnwrapFee(params, 1_200, 100);

        assertEq(fee, 0, "no fee when path never dips below target");
    }

    function testUnwrapAboveTargetCrossesBelowTarget() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 fee = lib.quoteUnwrapFee(params, 1_200, 500);

        assertEq(fee, 5, "fee should apply only to below-target segment");
    }

    function testUnwrapFeeOverdrawStartsFromZero() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 fee = lib.quoteUnwrapFee(params, 100, 150);

        assertEq(fee, 60, "over-withdraw should charge from zero-liquidity path");
    }

    function testUnwrapFeeOverdrawAboveTargetChargesShortfallPlusCurve() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 fee = lib.quoteUnwrapFee(params, 1_200, 1_500);

        assertEq(fee, 350, "over-withdraw should add shortfall plus full curve fee");
    }

    function testUnwrapFeeOverdrawCapsAtAmountWhenFeeExceedsLiquidity() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1, k: 100_000});

        uint256 fee = lib.quoteUnwrapFee(params, 1, 2);

        assertEq(fee, 2, "over-withdraw should cap fee at amount");
    }

    function testWrapRewardGracefullyHandlesHugeParams() public view {
        uint256 tooLargeTarget = uint256(type(uint128).max) + 1;
        IncentiveLib.FeeParams memory params =
            IncentiveLib.FeeParams({targetLiquidity: tooLargeTarget, k: TEN_PERCENT_K});

        uint256 reward = lib.quoteWrapReward(params, type(uint256).max, type(uint256).max, type(uint256).max);

        assertEq(reward, 0, "wrap reward should not revert and return zero when params are too large");
    }

    function testUnwrapFeeGracefullyHandlesHugeK() public view {
        uint256 safeTarget = 1_000;
        uint256 tooLargeK = type(uint256).max;
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: safeTarget, k: tooLargeK});

        uint256 fee = lib.quoteUnwrapFee(params, 1_000, 500);

        assertEq(fee, 0, "unwrap fee should not revert and return zero when k is too large");
    }

    function testUnwrapFeeIsCappedAtAmount() public view {
        // With small T and large k, raw fee would exceed amount; it should be capped to amount.
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1, k: 100_000});

        uint256 fee = lib.quoteUnwrapFee(params, 1, 1);

        assertEq(fee, 1, "unwrap fee should never exceed the requested amount");
    }
}
