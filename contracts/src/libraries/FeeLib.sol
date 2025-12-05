// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/// @notice Pure math helpers for reward + rebalancing fee calculations.
library FeeLib {
    uint256 internal constant BPS = 10_000;
    uint256 internal constant WAD = 1e18;

    struct RewardParams {
        uint256 liquiditySlopeBps;
    }

    struct FeeParams {
        uint256 lambda1Bps; // Marginal fee at the Tier-2 boundary (δ2).
        uint256 lambda2Bps; // Incremental marginal fee as balance moves below δ2.
        uint256 delta1Bps; // Utilization threshold (in bps of lTarget) above which rebalancing fee is 0.
        uint256 delta2Bps; // Lower threshold where the second slope begins.
    }

    function quoteWrap(uint256 amount, uint256 liquidityBefore, uint256 lTarget, RewardParams memory params)
        internal
        pure
        returns (uint256 rewardAmount)
    {
        if (amount == 0 || lTarget == 0) {
            return 0;
        }

        if (liquidityBefore >= lTarget) {
            return 0;
        }

        uint256 rewardable = amount;
        uint256 remainingToTarget = lTarget - liquidityBefore;
        if (rewardable > remainingToTarget) {
            rewardable = remainingToTarget;
        }

        // Integral over x in [0, rewardable] of (1 - (b + x)/B) dx
        // = rewardable * (B - b)/B - rewardable^2 / (2B)
        uint256 firstTerm = (rewardable * (lTarget - liquidityBefore)) / lTarget;
        uint256 secondTerm = (rewardable * rewardable) / (2 * lTarget);
        uint256 integral = firstTerm > secondTerm ? firstTerm - secondTerm : 0;

        rewardAmount = (params.liquiditySlopeBps * integral) / BPS;
    }

    function quoteUnwrap(uint256 amount, uint256 liquidityBefore, uint256 lTarget, FeeParams memory feeParams)
        internal
        pure
        returns (uint256 feeAmount)
    {
        if (amount == 0 || lTarget == 0) {
            return 0;
        }

        uint256 delta1Balance = (lTarget * feeParams.delta1Bps) / BPS;
        uint256 delta2Balance = (lTarget * feeParams.delta2Bps) / BPS;

        uint256 processAmount = amount > liquidityBefore ? liquidityBefore : amount;

        uint256 t1 = liquidityBefore > delta1Balance ? liquidityBefore - delta1Balance : 0;
        uint256 t2 = liquidityBefore > delta2Balance ? liquidityBefore - delta2Balance : 0;

        if (processAmount > t1 && delta1Balance > delta2Balance && delta2Balance > 0) {
            uint256 upper1 = processAmount < t2 ? processAmount : t2;
            if (upper1 > t1) {
                uint256 dx1 = upper1 - t1;
                uint256 span1 = delta1Balance - delta2Balance;
                uint256 endRate = (feeParams.lambda1Bps * dx1) / span1;
                feeAmount += (dx1 * endRate) / (2 * BPS);
            }

            if (processAmount > t2) {
                uint256 upper2 = processAmount < liquidityBefore ? processAmount : liquidityBefore;
                if (upper2 > t2) {
                    uint256 dx2 = upper2 - t2;
                    feeAmount += (dx2 * feeParams.lambda1Bps) / BPS;
                    feeAmount += (feeParams.lambda2Bps * dx2 * dx2) / (2 * delta2Balance * BPS);
                }
            }
        }

        if (feeAmount > amount) {
            feeAmount = amount;
        }
    }
}
