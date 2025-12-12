// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/**
 * @title IncentiveLib
 * @notice
 * A pure math library for calculating
 *  - rewards on deposits ("wrap"), and
 *  - fees on withdrawals ("unwrap")
 * for a single-asset liquidity pool.
 *
 * ---------------------------------------------------------------------------
 * Conceptual model
 * ---------------------------------------------------------------------------
 *
 * Think of the pool's total liquidity as a point on a horizontal axis:
 *
 *      0 ---------------- L ---------------------- T ----------->
 *
 * - L  : current liquidity
 * - T  : target liquidity (configured in FeeParams)
 * - k  : incentive strength coefficient (configured in FeeParams), stored in
 *        basis points where 1 bps = 0.01% (k = 10_000 => coefficient 1.0).
 *
 * The pool wants to:
 *  - strongly encourage deposits when liquidity is low, and
 *  - gradually turn off incentives as liquidity approaches T.
 *
 * To do this, we define a "reward/fee density" along the liquidity axis:
 *
 *   density(x) = k * (1 - x / T)   for 0 <= x < T
 *              = 0                 for x >= T
 *
 * Properties of this density:
 *  - Highest when x is near 0 (very low liquidity).
 *  - Decreases linearly as x increases.
 *  - Reaches exactly 0 at x = T.
 *  - Is 0 for any x >= T, so once liquidity is at or above T, the pool
 *    is neutral (no extra rewards or fees from this mechanism).
 *
 * ---------------------------------------------------------------------------
 * Deposit (wrap) rewards as area under the curve
 * ---------------------------------------------------------------------------
 *
 * When someone deposits an amount `amount`, liquidity moves from:
 *
 *   L_start = L
 *   L_end   = L + amount
 *
 * But we only care about the part of this movement that is below T, so:
 *
 *   D = min(L_start, T)
 *   U = min(L_end,   T)
 *
 * The continuous (ideal, real-valued) reward is the "area under the density
 * curve" between D and U:
 *
 *   reward_continuous = ∫[x=D..U] density(x) dx
 *
 * For the linear density defined above, this integral has a closed form:
 *
 *   reward_continuous = k / (2T) * [ (T - D)^2 - (T - U)^2 ]
 *
 * On-chain, we compute this value in integer arithmetic and round DOWN
 * (floor) to ensure we never overpay due to rounding.
 *
 * Finally, this raw reward is capped by `feeSurplus` passed in by the caller:
 *
 *   reward = min(reward_raw, feeSurplus)
 *
 * This guarantees that the caller can safely subtract `reward` from its
 * fee surplus without underflow.
 *
 * ---------------------------------------------------------------------------
 * Withdrawal (unwrap) fees as area under the same curve
 * ---------------------------------------------------------------------------
 *
 * When someone withdraws an amount `amount`, liquidity moves from:
 *
 *   L_start = L
 *   L_end   = L - amount
 *
 * Again, we only care about the part of this movement that is below T. So:
 *
 *   A = min(L_start,        T)
 *   B = min(L_start - amount, T)
 *
 * (note that B <= A in the relevant region).
 *
 * The continuous (ideal) fee is the area under the same density curve
 * between B and A:
 *
 *   fee_continuous = ∫[x=B..A] density(x) dx
 *
 * which has the closed form:
 *
 *   fee_continuous = k / (2T) * [ (T - B)^2 - (T - A)^2 ]
 *
 * On-chain, we compute this value in integer arithmetic and round UP (ceil)
 * to ensure we never undercharge due to rounding:
 *
 *   fee = ceil(fee_continuous)
 *
 * Special protection:
 *  - If `amount > L`, the requested withdrawal is larger than the current
 *    liquidity. In this case, the library returns:
 *
 *      fee = amount
 *
 *    This allows the caller to treat such a withdrawal as "net zero" for
 *    the user (fee equals the requested amount) and prevents liquidity
 *    underflow in the caller contract.
 *
 * ---------------------------------------------------------------------------
 * No-arbitrage and rounding direction
 * ---------------------------------------------------------------------------
 *
 * In the continuous (real-valued) model:
 *  - If you move from L to L + amount (deposit), and then back from
 *    L + amount to L (withdraw), the total reward and total fee are
 *    exactly equal:
 *
 *      reward_continuous = fee_continuous  (same path, reversed)
 *
 *  - Similarly for withdraw then deposit along the same path.
 *
 * On-chain, we deliberately:
 *  - round rewards DOWN (floor), and
 *  - round fees UP   (ceil).
 *
 * Therefore, for any path:
 *
 *   collected_fee >= paid_reward
 *
 * This means an attacker cannot profit by performing atomic deposit/withdraw
 * cycles that only move liquidity along the same path and back.
 *
 * ---------------------------------------------------------------------------
 * Usage
 * ---------------------------------------------------------------------------
 *
 * The library is stateless. A typical calling contract stores:
 *
 *   FeeParams params;
 *   uint256   liquidity;   // current total liquidity
 *   uint256   feeSurplus;  // accumulated surplus from past fees
 *
 * And uses:
 *
 *   uint256 reward = IncentiveLib.quoteWrapReward(params, liquidity, feeSurplus, amount);
 *   uint256 fee    = IncentiveLib.quoteUnwrapFee(params, liquidity, amount);
 *
 * The caller is responsible for:
 *  - updating its own `liquidity` and `feeSurplus` storage,
 *  - performing any token transfers,
 *  - enforcing any additional business logic.
 */
library IncentiveLib {
    /**
     * @notice Parameters that define the fee/reward curve.
     */
    struct FeeParams {
        /// @notice Target liquidity T where incentives fade to zero.
        uint256 targetLiquidity;
        /// @notice Incentive strength coefficient k, expressed in basis points (1 = 0.01%).
        uint256 k;
    }

    /// @notice Basis points denominator for k (1 bps = 0.01% = 1 / 10_000).
    uint256 internal constant K_BPS_DENOM = 10_000;
    /// @notice Largest T such that intermediate squares fit in uint256.
    uint256 internal constant MAX_TARGET_LIQUIDITY = type(uint128).max;

    /*//////////////////////////////////////////////////////////////
                           PUBLIC API (INTERNAL)
    //////////////////////////////////////////////////////////////*/

    /**
     * @notice Compute deposit reward for adding `amount` liquidity.
     * @dev
     * - Uses the continuous density described in the library header.
     * - Rounds DOWN (floor).
     * - Caps the result by `feeSurplus`.
     */
    function quoteWrapReward(
        FeeParams memory params,
        uint256 liquidity,
        uint256 feeSurplus,
        uint256 amount
    ) internal pure returns (uint256 wrapReward) {
        uint256 raw = _rawWrapReward(liquidity, params.targetLiquidity, params.k, amount);

        if (raw > feeSurplus) return feeSurplus;
        return raw;
    }

    /**
     * @notice Compute withdrawal fee for removing `amount` liquidity.
     * @dev
     * - Uses the same continuous density described in the library header.
     * - Rounds UP (ceil).
     * - If `amount > liquidity`, returns `amount` to allow "net-zero" withdrawal.
     */
    function quoteUnwrapFee(
        FeeParams memory params,
        uint256 liquidity,
        uint256 amount
    ) internal pure returns (uint256 unwrapFee) {
        uint256 raw = _rawUnwrapFee(liquidity, params.targetLiquidity, params.k, amount);
        return raw > amount ? amount : raw;
    }

    /*//////////////////////////////////////////////////////////////
                         INTERNAL PURE MATH HELPERS
    //////////////////////////////////////////////////////////////*/

    function _rawWrapReward(
        uint256 L,
        uint256 T,
        uint256 k_,
        uint256 amount
    ) internal pure returns (uint256 rewardRaw) {
        if (T == 0 || amount == 0) {
            return 0;
        }
        if (T > MAX_TARGET_LIQUIDITY) {
            return 0;
        }
        uint256 maxK = type(uint256).max / T / T;
        if (k_ > maxK) {
            return 0;
        }

        // Portion of the path within [0, T).
        uint256 start = L < T ? L : T; // start, capped at T
        uint256 end;
        if (amount > type(uint256).max - L) {
            end = type(uint256).max; // saturate on overflow
        } else {
            end = L + amount; // end before capping
        }
        if (end > T) end = T; // cap at T

        if (end <= start) {
            return 0;
        }

        // reward_continuous ∝ (T - D)^2 - (T - U)^2
        uint256 distanceToTargetStart = T - start; // T >= start
        uint256 distanceToTargetEnd = T - end; // end <= T

        uint256 startDistanceSq = distanceToTargetStart * distanceToTargetStart;
        uint256 endDistanceSq = distanceToTargetEnd * distanceToTargetEnd;
        uint256 diffSquare = startDistanceSq - endDistanceSq; // non-negative because start distance >= end distance

        // k is provided in basis points, so divide by K_BPS_DENOM.
        uint256 numerator = k_ * diffSquare;
        uint256 denominator = 2 * T * K_BPS_DENOM;

        // Floor (round down).
        rewardRaw = numerator / denominator;
    }

    function _rawUnwrapFee(
        uint256 L,
        uint256 T,
        uint256 k_,
        uint256 amount
    ) internal pure returns (uint256 feeRaw) {
        if (amount == 0) {
            return 0;
        }

        // If requested withdrawal exceeds liquidity, charge full amount as fee.
        if (amount > L) {
            return amount;
        }

        if (T == 0) {
            return 0;
        }
        if (T > MAX_TARGET_LIQUIDITY) {
            return 0;
        }

        if (L == 0) {
            return 0; // unreachable in normal use, kept as a safety guard
        }
        uint256 maxK = type(uint256).max / T / T;
        if (k_ > maxK) {
            return 0;
        }

        // Portion of the path within [0, T).
        uint256 start = L < T ? L : T; // start (higher), capped at T
        uint256 endRaw = L - amount; // end (lower) before capping
        uint256 end = endRaw < T ? endRaw : T; // cap at T

        if (start <= end) {
            return 0;
        }

        // fee_continuous ∝ (T - B)^2 - (T - A)^2
        uint256 distanceToTargetStart = T - start; // start <= T
        uint256 distanceToTargetEnd = T - end; // end <= T

        uint256 startDistanceSq = distanceToTargetStart * distanceToTargetStart;
        uint256 endDistanceSq = distanceToTargetEnd * distanceToTargetEnd;
        uint256 diffSquare = endDistanceSq - startDistanceSq; // non-negative because end distance >= start distance

        // k is provided in basis points, so divide by K_BPS_DENOM.
        uint256 numerator = k_ * diffSquare;
        uint256 denominator = 2 * T * K_BPS_DENOM;

        // Ceil (round up).
        feeRaw = _ceilDiv(numerator, denominator);
    }

    function _ceilDiv(uint256 a, uint256 b) internal pure returns (uint256 c) {
        if (a == 0) return 0;
        uint256 q = a / b;
        uint256 r = a % b;
        return r == 0 ? q : q + 1;
    }
}
