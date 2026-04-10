// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.33;

import {Test} from "forge-std/Test.sol";
import {IncentiveLib} from "../../src/libraries/IncentiveLib.sol";

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

    function validateFeeParams(IncentiveLib.FeeParams calldata params) external pure {
        IncentiveLib.validateFeeParams(params);
    }
}

contract IncentiveLibTest is Test {
    IncentiveLibHarness internal lib;
    uint256 internal constant TEN_PERCENT_K = 1_000; // 0.1 in basis points terms
    uint256 internal constant MAX_K = 10_000;

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

    // =========================================================================
    // validateFeeParams tests
    // =========================================================================

    function testValidateFeeParamsAcceptsValidParams() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});
        lib.validateFeeParams(params);
    }

    function testValidateFeeParamsAcceptsZeroTarget() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 0, k: TEN_PERCENT_K});
        lib.validateFeeParams(params);
    }

    function testValidateFeeParamsAcceptsZeroK() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: 0});
        lib.validateFeeParams(params);
    }

    function testValidateFeeParamsAcceptsMaxK() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: MAX_K});
        lib.validateFeeParams(params);
    }

    function testValidateFeeParamsRevertsOnKOverflowGuard() public {
        // For very large targets, `maxK = type(uint256).max / T / T` becomes extremely small (often 0 or 1).
        // Even if k <= 10_000, we must revert to avoid overflow in k * (T - x)^2.
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: type(uint128).max, k: 2});

        vm.expectRevert(IncentiveLib.InvalidK.selector);
        lib.validateFeeParams(params);
    }

    function testValidateFeeParamsRevertsOnTooLargeTarget() public {
        uint256 tooLargeTarget = uint256(type(uint128).max) + 1;
        IncentiveLib.FeeParams memory params =
            IncentiveLib.FeeParams({targetLiquidity: tooLargeTarget, k: TEN_PERCENT_K});

        vm.expectRevert(IncentiveLib.InvalidTarget.selector);
        lib.validateFeeParams(params);
    }

    function testValidateFeeParamsRevertsOnKExceedingBpsDenom() public {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: 10_001});

        vm.expectRevert(IncentiveLib.InvalidK.selector);
        lib.validateFeeParams(params);
    }

    // =========================================================================
    // Edge case tests: zero values
    // =========================================================================

    function testWrapRewardWithZeroAmount() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 reward = lib.quoteWrapReward(params, 500, type(uint256).max, 0);

        assertEq(reward, 0, "zero amount should yield zero reward");
    }

    function testUnwrapFeeWithZeroAmount() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 fee = lib.quoteUnwrapFee(params, 500, 0);

        assertEq(fee, 0, "zero amount should yield zero fee");
    }

    function testWrapRewardWithZeroTarget() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 0, k: TEN_PERCENT_K});

        uint256 reward = lib.quoteWrapReward(params, 500, type(uint256).max, 100);

        assertEq(reward, 0, "zero target should yield zero reward");
    }

    function testUnwrapFeeWithZeroTarget() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 0, k: TEN_PERCENT_K});

        uint256 fee = lib.quoteUnwrapFee(params, 500, 100);

        assertEq(fee, 0, "zero target should yield zero fee");
    }

    function testWrapRewardWithZeroK() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: 0});

        uint256 reward = lib.quoteWrapReward(params, 0, type(uint256).max, 500);

        assertEq(reward, 0, "zero k should yield zero reward");
    }

    function testUnwrapFeeWithZeroK() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: 0});

        uint256 fee = lib.quoteUnwrapFee(params, 500, 100);

        assertEq(fee, 0, "zero k should yield zero fee");
    }

    function testUnwrapFeeWithZeroLiquidity() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 fee = lib.quoteUnwrapFee(params, 0, 100);

        assertEq(fee, 100, "zero liquidity should charge full amount");
    }

    function testWrapRewardWithZeroFeeSurplus() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 reward = lib.quoteWrapReward(params, 0, 0, 500);

        assertEq(reward, 0, "zero fee surplus should yield zero reward");
    }

    function testWrapRewardSaturatingAddStillCapsToTarget() public view {
        // Triggers the `L + amount` overflow branch; the path is still capped at T.
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: MAX_K});

        uint256 reward = lib.quoteWrapReward(params, 900, type(uint256).max, type(uint256).max);

        // With k = 1.0 and T = 1000, the area from 900 -> 1000 is exactly 5.
        assertEq(reward, 5, "overflowing amount should still compute reward up to target");
    }

    // =========================================================================
    // Symmetry tests
    // =========================================================================

    function testWrapAndUnwrapSymmetryAtTarget() public view {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000, k: TEN_PERCENT_K});

        uint256 wrapReward = lib.quoteWrapReward(params, 0, type(uint256).max, 1_000);
        uint256 unwrapFee = lib.quoteUnwrapFee(params, 1_000, 1_000);

        assertEq(wrapReward, unwrapFee, "wrap 0->T and unwrap T->0 should be symmetric");
    }

    // =========================================================================
    // Fuzz tests
    // =========================================================================

    function testFuzzUnwrapFeeNeverExceedsAmountWithValidatedParams(
        uint256 targetLiquidity,
        uint256 k,
        uint256 liquidity,
        uint256 amount
    ) public view {
        targetLiquidity = bound(targetLiquidity, 1, type(uint128).max);
        uint256 maxK = type(uint256).max / targetLiquidity / targetLiquidity;
        k = bound(k, 0, _min(MAX_K, maxK));
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: targetLiquidity, k: k});
        lib.validateFeeParams(params);

        uint256 fee = lib.quoteUnwrapFee(params, liquidity, amount);
        assertLe(fee, amount, "fee should never exceed amount for validated params");
    }

    function testFuzzWrapRewardNeverExceedsAmountWithValidatedParams(
        uint256 targetLiquidity,
        uint256 k,
        uint256 liquidity,
        uint256 feeSurplus,
        uint256 amount
    ) public view {
        targetLiquidity = bound(targetLiquidity, 1, type(uint128).max);
        uint256 maxK = type(uint256).max / targetLiquidity / targetLiquidity;
        k = bound(k, 0, _min(MAX_K, maxK));
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: targetLiquidity, k: k});
        lib.validateFeeParams(params);

        uint256 reward = lib.quoteWrapReward(params, liquidity, feeSurplus, amount);
        assertLe(reward, feeSurplus, "reward should never exceed fee surplus");
        assertLe(reward, amount, "reward should never exceed amount for validated params");
    }

    function testFuzzWrapRewardNeverExceedsFeeSurplus(uint256 liquidity, uint256 feeSurplus, uint256 amount)
        public
        view
    {
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000_000, k: TEN_PERCENT_K});

        uint256 reward = lib.quoteWrapReward(params, liquidity, feeSurplus, amount);

        assertLe(reward, feeSurplus, "reward should never exceed fee surplus");
    }

    function testFuzzUnwrapFeeNeverExceedsAmount(uint256 liquidity, uint256 amount) public view {
        // Bound inputs to avoid zero division edge cases
        liquidity = bound(liquidity, 1, type(uint128).max);
        amount = bound(amount, 1, type(uint128).max);

        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000_000, k: TEN_PERCENT_K});

        uint256 fee = lib.quoteUnwrapFee(params, liquidity, amount);

        assertLe(fee, amount, "fee should never exceed amount");
    }

    function testFuzzWrapRewardDoesNotRevert(
        uint256 targetLiquidity,
        uint256 k,
        uint256 liquidity,
        uint256 feeSurplus,
        uint256 amount
    ) public view {
        // Use safe bounds for valid params
        targetLiquidity = bound(targetLiquidity, 0, type(uint128).max);
        k = bound(k, 0, 10_000);

        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: targetLiquidity, k: k});

        // Should not revert
        lib.quoteWrapReward(params, liquidity, feeSurplus, amount);
    }

    function testFuzzUnwrapFeeDoesNotRevert(uint256 targetLiquidity, uint256 k, uint256 liquidity, uint256 amount)
        public
        view
    {
        // Use safe bounds for valid params
        targetLiquidity = bound(targetLiquidity, 0, type(uint128).max);
        k = bound(k, 0, 10_000);

        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: targetLiquidity, k: k});

        // Should not revert
        lib.quoteUnwrapFee(params, liquidity, amount);
    }

    function _min(uint256 a, uint256 b) private pure returns (uint256) {
        return a < b ? a : b;
    }
}
