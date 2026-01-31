//! Proof of Innocence circuit for OFAC non-membership proofs.
//!
//! This circuit proves that a set of transfers to a recipient did not originate
//! from sanctioned addresses (e.g., OFAC list) without revealing transaction details.
//!
//! # Exclusion Tree
//!
//! The OFAC sanctions list is committed using an "exclusion tree" - a Merkle tree
//! of sorted disjoint (start, end) pairs representing the gaps between sanctioned
//! addresses. Each leaf is computed as `poseidon2(start, end)`.
//!
//! If the sanctions list contains k addresses, the exclusion tree has k + 1 gaps
//! (one before the first address, one between each consecutive pair, and one after
//! the last).
//!
//! # Non-Membership Proof
//!
//! To prove an address is NOT sanctioned, we show it lies within one of these gaps
//! by proving `start < from_address < end` and verifying the Merkle proof for that
//! gap against the trusted OFAC root.

use crate::{
    circuits::constants::{ADDRESS_BIT_LENGTH, BYTES31_BIT_LENGTH},
    utils::{
        poseidon::gadgets::{poseidon2_var, CircomCRHParametersVar},
        tree::gadgets::merkle::{enforce_bit_length, merkle_root_from_leaf, to_bits_le_limited},
    },
};
use ark_crypto_primitives::sponge::Absorb;
use ark_ff::PrimeField;
use ark_r1cs_std::{
    boolean::Boolean,
    eq::EqGadget,
    fields::{FieldVar, fp::FpVar},
};
use ark_relations::gr1cs::SynthesisError;

type InnocenceStepResult<F> = Result<FpVar<F>, SynthesisError>;

/// Single step of the Proof of Innocence Nova circuit.
///
/// For each transfer to a recipient, proves that the sender (`from_address`)
/// is NOT in the OFAC sanctions list by verifying an exclusion proof.
///
/// # Arguments
///
/// * `poseidon2_params` - Poseidon config for 2-to-1 hashing (gap leaf and Merkle tree)
/// * `ofac_root` - Root of the OFAC exclusion tree (public, constant across steps)
/// * `from_address` - Sender address to prove is not sanctioned
/// * `prev_total_value` - Running sum of transfer values from previous steps
/// * `is_dummy` - Whether this is a padding step (skips verification, subtracts value)
/// * `value` - This transfer's value
/// * `start` - Lower bound of the exclusion gap containing `from_address`
/// * `end` - Upper bound of the exclusion gap containing `from_address`
/// * `gap_index` - Position of this gap leaf in the exclusion tree
/// * `siblings` - Merkle proof siblings for the gap leaf
///
/// # Returns
///
/// The new accumulated total value.
///
/// # Constraints
///
/// 1. Range checks on all inputs
/// 2. Exclusion proof: `start < from_address < end`
/// 3. Gap leaf hash: `poseidon2(start, end)`
/// 4. Merkle proof verification against `ofac_root`
/// 5. Value accumulation: `new_total = prev_total + value` (or `- value` for dummy)
#[allow(clippy::too_many_arguments)]
pub fn innocence_step<F, const DEPTH: usize>(
    poseidon2_params: &CircomCRHParametersVar<F>,
    ofac_root: &FpVar<F>,
    from_address: &FpVar<F>,
    prev_total_value: &FpVar<F>,
    is_dummy: &Boolean<F>,
    value: &FpVar<F>,
    start: &FpVar<F>,
    end: &FpVar<F>,
    gap_index: &FpVar<F>,
    siblings: &[FpVar<F>],
) -> InnocenceStepResult<F>
where
    F: PrimeField + Absorb,
{
    assert_eq!(siblings.len(), DEPTH);

    // Range checks
    enforce_bit_length(from_address, ADDRESS_BIT_LENGTH)?;
    enforce_bit_length(start, ADDRESS_BIT_LENGTH)?;
    enforce_bit_length(end, ADDRESS_BIT_LENGTH)?;
    enforce_bit_length(value, BYTES31_BIT_LENGTH)?;
    enforce_bit_length(gap_index, DEPTH)?;

    // 1. Verify exclusion: start < from_address < end
    //    This proves from_address lies in a gap (not in the sanctions list)
    //    Only enforce if this is a real step (not dummy)

    // We need to conditionally enforce the ordering constraints.
    // For real steps: enforce start < from_address < end
    // For dummy steps: skip the check
    //
    // We do this by computing the differences and enforcing they're zero when is_real
    let one = FpVar::<F>::constant(F::one());
    let is_dummy_fp: FpVar<F> = is_dummy.clone().into();
    let is_real_fp = one.clone() - is_dummy_fp.clone();

    // Enforce ordering constraints only for real steps
    // We use enforce_strict_less_than which does range checks, so we only call it
    // if we're in a real step. For dummy steps, we still need the circuit to be
    // satisfiable, so we use conditional constraints.
    //
    // Approach: Compute what the ordering check would produce, then conditionally
    // enforce it. The enforce_strict_less_than already does range checks, which we
    // did above, so we can use a simpler conditional approach here.

    // For the ordering, we check:
    // - start < from_address: from_address - start - 1 >= 0 (fits in ADDRESS_BIT_LENGTH bits)
    // - from_address < end: end - from_address - 1 >= 0 (fits in ADDRESS_BIT_LENGTH bits)
    //
    // We compute these differences and enforce they fit in ADDRESS_BIT_LENGTH bits
    // only when is_real.

    // Compute diff1 = from_address - start - 1 (should be >= 0 if start < from_address)
    let diff1 = from_address.clone() - start.clone() - one.clone();

    // Compute diff2 = end - from_address - 1 (should be >= 0 if from_address < end)
    let diff2 = end.clone() - from_address.clone() - one.clone();

    // For real steps, these differences must fit in ADDRESS_BIT_LENGTH bits (i.e., be non-negative)
    // For dummy steps, we don't care about the actual values
    //
    // We enforce this by: if is_real, then diff must be in range
    // This is done by checking that diff * is_real fits in range, but that's tricky.
    //
    // Alternative: Use the existing enforce_strict_less_than but make it conditional.
    // Since we can't easily make it conditional, let's use a different approach:
    //
    // We'll enforce the bit decomposition only for real steps by using select.
    // If is_dummy, we replace the value with a known-good value (like 0) before range checking.

    // For dummy steps, replace diffs with 0 (which trivially passes range check)
    let zero = FpVar::<F>::constant(F::zero());
    let diff1_checked = is_dummy.select(&zero, &diff1)?;
    let diff2_checked = is_dummy.select(&zero, &diff2)?;

    // Now enforce range (these must be non-negative and fit in ADDRESS_BIT_LENGTH bits)
    enforce_bit_length(&diff1_checked, ADDRESS_BIT_LENGTH)?;
    enforce_bit_length(&diff2_checked, ADDRESS_BIT_LENGTH)?;

    // 2. Compute gap leaf hash: poseidon2(start, end)
    let gap_leaf = poseidon2_var(poseidon2_params, start, end)?;

    // 3. Verify Merkle proof for gap leaf against ofac_root
    let index_bits = to_bits_le_limited(gap_index, DEPTH)?;
    let computed_root = merkle_root_from_leaf(poseidon2_params, &gap_leaf, &index_bits, siblings)?;

    // Conditional constraint: if real (not dummy), computed root must match ofac_root
    let root_diff = ofac_root.clone() - computed_root;
    (root_diff * is_real_fp.clone()).enforce_equal(&zero)?;

    // 4. Accumulate value
    //    Real step: add value to total
    //    Dummy step: subtract value from total (for batch padding, net zero effect)
    let two = F::from(2u64);
    let factor = one - is_dummy_fp * FpVar::<F>::constant(two);
    let new_total_value = prev_total_value.clone() + value.clone() * factor;
    enforce_bit_length(&new_total_value, BYTES31_BIT_LENGTH)?;

    Ok(new_total_value)
}

#[cfg(test)]
mod tests {
    use super::innocence_step;
    use crate::utils::poseidon::{
        gadgets::CircomCRHParametersVar,
        utils::{circom_poseidon2_config, poseidon2},
    };
    use ark_bn254::Fr;
    use ark_r1cs_std::{alloc::AllocVar, boolean::Boolean, eq::EqGadget, fields::fp::FpVar};
    use ark_relations::{
        gr1cs::{ConstraintSystem, SynthesisError},
        ns,
    };

    const DEPTH: usize = 4;

    /// Helper to build a simple exclusion tree for testing.
    /// Given a list of gap (start, end) pairs, computes the Merkle root.
    fn build_test_exclusion_tree(gaps: &[(Fr, Fr)]) -> Fr {
        let leaves: Vec<Fr> = gaps.iter().map(|(s, e)| poseidon2(*s, *e)).collect();

        // Pad to power of 2 if needed
        let mut padded_leaves = leaves.clone();
        while padded_leaves.len() < (1 << DEPTH) {
            padded_leaves.push(Fr::from(0u64));
        }

        // Build tree bottom-up
        let mut current_level = padded_leaves;
        while current_level.len() > 1 {
            let mut next_level = Vec::new();
            for chunk in current_level.chunks(2) {
                let hash = poseidon2(chunk[0], chunk[1]);
                next_level.push(hash);
            }
            current_level = next_level;
        }
        current_level[0]
    }

    /// Helper to get Merkle proof for a leaf at given index
    fn get_merkle_proof(gaps: &[(Fr, Fr)], index: usize) -> Vec<Fr> {
        let leaves: Vec<Fr> = gaps.iter().map(|(s, e)| poseidon2(*s, *e)).collect();

        // Pad to power of 2
        let mut padded_leaves = leaves.clone();
        while padded_leaves.len() < (1 << DEPTH) {
            padded_leaves.push(Fr::from(0u64));
        }

        let mut siblings = Vec::new();
        let mut current_level = padded_leaves;
        let mut current_index = index;

        for _ in 0..DEPTH {
            let sibling_index = if current_index % 2 == 0 {
                current_index + 1
            } else {
                current_index - 1
            };
            siblings.push(current_level[sibling_index]);

            // Build next level
            let mut next_level = Vec::new();
            for chunk in current_level.chunks(2) {
                let hash = poseidon2(chunk[0], chunk[1]);
                next_level.push(hash);
            }
            current_level = next_level;
            current_index /= 2;
        }

        siblings
    }

    #[test]
    fn innocence_step_accepts_valid_exclusion_proof() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;

        // Create a simple exclusion tree with 3 gaps:
        // Gap 0: [0, 100) - addresses 0-99 are NOT sanctioned
        // Gap 1: (100, 200) - addresses 101-199 are NOT sanctioned (100 and 200 are sanctioned)
        // Gap 2: (200, MAX) - addresses 201+ are NOT sanctioned
        let gaps = vec![
            (Fr::from(0u64), Fr::from(100u64)),
            (Fr::from(100u64), Fr::from(200u64)),
            (Fr::from(200u64), Fr::from(1000u64)),
        ];

        let ofac_root_value = build_test_exclusion_tree(&gaps);

        // Test: prove that address 150 is not sanctioned (lies in gap 1: 100 < 150 < 200)
        let from_address_value = Fr::from(150u64);
        let start_value = Fr::from(100u64);
        let end_value = Fr::from(200u64);
        let gap_index_value = Fr::from(1u64); // Gap 1
        let value_value = Fr::from(1000u64);
        let prev_total_value = Fr::from(500u64);
        let is_dummy_value = false;

        let siblings_values = get_merkle_proof(&gaps, 1);

        // Allocate variables
        let ofac_root = FpVar::<Fr>::new_witness(ns!(cs, "ofac_root"), || Ok(ofac_root_value))?;
        let from_address =
            FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_address_value))?;
        let prev_total =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(prev_total_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(is_dummy_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(start_value))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(end_value))?;
        let gap_index = FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || Ok(gap_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let new_total = innocence_step::<Fr, DEPTH>(
            &poseidon2_params,
            &ofac_root,
            &from_address,
            &prev_total,
            &is_dummy,
            &value,
            &start,
            &end,
            &gap_index,
            &siblings,
        )?;

        // Expected: prev_total + value = 500 + 1000 = 1500
        let expected_total = Fr::from(1500u64);
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_total))?;
        new_total.enforce_equal(&expected_var)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn innocence_step_rejects_sanctioned_address() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;

        // Same exclusion tree
        let gaps = vec![
            (Fr::from(0u64), Fr::from(100u64)),
            (Fr::from(100u64), Fr::from(200u64)),
            (Fr::from(200u64), Fr::from(1000u64)),
        ];

        let ofac_root_value = build_test_exclusion_tree(&gaps);

        // Test: try to prove address 100 is not sanctioned
        // But 100 is exactly at the boundary (sanctioned), so start < 100 < end should fail
        // Using gap 1: start=100, end=200, the check 100 < 100 fails
        let from_address_value = Fr::from(100u64);
        let start_value = Fr::from(100u64);
        let end_value = Fr::from(200u64);
        let gap_index_value = Fr::from(1u64);
        let value_value = Fr::from(1000u64);
        let prev_total_value = Fr::from(0u64);
        let is_dummy_value = false;

        let siblings_values = get_merkle_proof(&gaps, 1);

        let ofac_root = FpVar::<Fr>::new_witness(ns!(cs, "ofac_root"), || Ok(ofac_root_value))?;
        let from_address =
            FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_address_value))?;
        let prev_total =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(prev_total_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(is_dummy_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(start_value))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(end_value))?;
        let gap_index = FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || Ok(gap_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let new_total = innocence_step::<Fr, DEPTH>(
            &poseidon2_params,
            &ofac_root,
            &from_address,
            &prev_total,
            &is_dummy,
            &value,
            &start,
            &end,
            &gap_index,
            &siblings,
        )?;

        let expected_total = Fr::from(1000u64);
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_total))?;
        new_total.enforce_equal(&expected_var)?;

        // Should NOT be satisfied because from_address == start (not strictly greater)
        assert!(!cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn innocence_step_dummy_skips_verification() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;

        // For dummy steps, we can use invalid data - it should still satisfy
        let ofac_root_value = Fr::from(999999u64); // Invalid root
        let from_address_value = Fr::from(100u64); // Would be sanctioned
        let start_value = Fr::from(100u64); // Invalid: from == start
        let end_value = Fr::from(200u64);
        let gap_index_value = Fr::from(0u64);
        let value_value = Fr::from(50u64);
        let prev_total_value = Fr::from(100u64);
        let is_dummy_value = true; // DUMMY - skip verification

        let siblings_values = vec![Fr::from(0u64); DEPTH]; // Invalid siblings

        let ofac_root = FpVar::<Fr>::new_witness(ns!(cs, "ofac_root"), || Ok(ofac_root_value))?;
        let from_address =
            FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_address_value))?;
        let prev_total =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(prev_total_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(is_dummy_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(start_value))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(end_value))?;
        let gap_index = FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || Ok(gap_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let new_total = innocence_step::<Fr, DEPTH>(
            &poseidon2_params,
            &ofac_root,
            &from_address,
            &prev_total,
            &is_dummy,
            &value,
            &start,
            &end,
            &gap_index,
            &siblings,
        )?;

        // Dummy subtracts: prev_total - value = 100 - 50 = 50
        let expected_total = Fr::from(50u64);
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_total))?;
        new_total.enforce_equal(&expected_var)?;

        // Should be satisfied because is_dummy=true skips verification
        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn innocence_step_rejects_wrong_merkle_proof() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;

        let gaps = vec![
            (Fr::from(0u64), Fr::from(100u64)),
            (Fr::from(100u64), Fr::from(200u64)),
            (Fr::from(200u64), Fr::from(1000u64)),
        ];

        let ofac_root_value = build_test_exclusion_tree(&gaps);

        // Valid address in gap, but wrong Merkle proof (using proof for gap 0 instead of gap 1)
        let from_address_value = Fr::from(150u64);
        let start_value = Fr::from(100u64);
        let end_value = Fr::from(200u64);
        let gap_index_value = Fr::from(1u64);
        let value_value = Fr::from(1000u64);
        let prev_total_value = Fr::from(0u64);
        let is_dummy_value = false;

        // Wrong proof - using index 0's proof for index 1's leaf
        let siblings_values = get_merkle_proof(&gaps, 0);

        let ofac_root = FpVar::<Fr>::new_witness(ns!(cs, "ofac_root"), || Ok(ofac_root_value))?;
        let from_address =
            FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_address_value))?;
        let prev_total =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(prev_total_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(is_dummy_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(start_value))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(end_value))?;
        let gap_index = FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || Ok(gap_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let new_total = innocence_step::<Fr, DEPTH>(
            &poseidon2_params,
            &ofac_root,
            &from_address,
            &prev_total,
            &is_dummy,
            &value,
            &start,
            &end,
            &gap_index,
            &siblings,
        )?;

        let expected_total = Fr::from(1000u64);
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_total))?;
        new_total.enforce_equal(&expected_var)?;

        // Should NOT be satisfied because Merkle proof doesn't match
        assert!(!cs.is_satisfied().unwrap());
        Ok(())
    }
}
