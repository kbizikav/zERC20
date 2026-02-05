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
//!
//! # Recipient Binding (PoW)
//!
//! Each transfer is bound to the recipient via the burn address PoW mechanism.
//! The prover must provide a `secret` such that `derive(recipient, secret)` satisfies
//! the PoW constraint. This prevents an attacker from using another user's transfers
//! in their proof - the secret valid for Bob's recipient won't satisfy PoW for Carol's.

use crate::{
    circuits::{
        burn_address::burn_address_var,
        constants::{ADDRESS_BIT_LENGTH, BYTES31_BIT_LENGTH},
    },
    utils::{
        poseidon::gadgets::{CircomCRHParametersVar, poseidon2_var},
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
use core::ops::Not;

type InnocenceStepResult<F> = Result<FpVar<F>, SynthesisError>;

/// Single step of the Proof of Innocence Nova circuit.
///
/// For each transfer to a recipient, proves:
/// 1. **Recipient binding**: The transfer belongs to this recipient (via burn address PoW)
/// 2. **Non-membership**: The sender (`from_address`) is NOT in the OFAC sanctions list
///
/// # Arguments
///
/// * `poseidon2_params` - Poseidon config for 2-to-1 hashing (gap leaf and Merkle tree)
/// * `poseidon3_params` - Poseidon config for 3-to-1 hashing (burn address derivation)
/// * `ofac_root` - Root of the OFAC exclusion tree (public, constant across steps)
/// * `recipient` - Hash of GeneralRecipient (public, constant across steps)
/// * `from_address` - Sender address to prove is not sanctioned
/// * `prev_total_value` - Running sum of transfer values from previous steps
/// * `is_dummy` - Whether this is a padding step (skips verification, subtracts value)
/// * `value` - This transfer's value
/// * `secret` - Secret used to derive the burn address (proves transfer belongs to recipient)
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
/// 1. **Recipient binding**: `burn_address = derive(recipient, secret)` with PoW check
/// 2. Range checks on all inputs
/// 3. Exclusion proof: `start < from_address < end`
/// 4. Gap leaf hash: `poseidon2(start, end)`
/// 5. Merkle proof verification against `ofac_root`
/// 6. Value accumulation: `new_total = prev_total + value` (or `- value` for dummy)
#[allow(clippy::too_many_arguments)]
pub fn innocence_step<F, const DEPTH: usize>(
    poseidon2_params: &CircomCRHParametersVar<F>,
    poseidon3_params: &CircomCRHParametersVar<F>,
    ofac_root: &FpVar<F>,
    recipient: &FpVar<F>,
    from_address: &FpVar<F>,
    prev_total_value: &FpVar<F>,
    is_dummy: &Boolean<F>,
    value: &FpVar<F>,
    secret: &FpVar<F>,
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

    let one = FpVar::<F>::constant(F::one());
    let zero = FpVar::<F>::constant(F::zero());
    let is_dummy_fp: FpVar<F> = is_dummy.clone().into();
    let is_real_fp = one.clone() - is_dummy_fp.clone();

    // Compute should_constrain for conditional checks
    let should_constrain = is_dummy.not();

    // 1. RECIPIENT BINDING (PoW)
    //    Verify burn_address = derive(recipient, secret) with PoW constraint.
    //    This proves the transfer was intended for this recipient.
    //    The PoW check is conditional on should_constrain (skipped for dummy steps).
    //    Burn address is left unused by design. It is no further of use in the circuit.
    let _burn_address = burn_address_var(poseidon3_params, recipient, secret, &should_constrain)?;

    // 2. EXCLUSION PROOF: Verify start < from_address < end
    //    This proves from_address lies in a gap (not in the sanctions list)
    //    Only enforce if this is a real step (not dummy)
    //
    //    We check:
    //    - start < from_address: (from_address - start - 1) fits in ADDRESS_BIT_LENGTH bits
    //    - from_address < end: (end - from_address - 1) fits in ADDRESS_BIT_LENGTH bits

    // Compute diff1 = from_address - start - 1 (>= 0 iff start < from_address)
    let diff1 = from_address.clone() - start.clone() - one.clone();
    // Compute diff2 = end - from_address - 1 (>= 0 iff from_address < end)
    let diff2 = end.clone() - from_address.clone() - one.clone();

    // For dummy steps, replace diffs with 0 (which trivially passes range check)
    let diff1_checked = is_dummy.select(&zero, &diff1)?;
    let diff2_checked = is_dummy.select(&zero, &diff2)?;

    // Enforce range (these must be non-negative and fit in ADDRESS_BIT_LENGTH bits)
    enforce_bit_length(&diff1_checked, ADDRESS_BIT_LENGTH)?;
    enforce_bit_length(&diff2_checked, ADDRESS_BIT_LENGTH)?;

    // 3. GAP LEAF HASH: Compute poseidon2(start, end)
    let gap_leaf = poseidon2_var(poseidon2_params, start, end)?;

    // 4. MERKLE PROOF: Verify gap leaf against ofac_root
    let index_bits = to_bits_le_limited(gap_index, DEPTH)?;
    let computed_root = merkle_root_from_leaf(poseidon2_params, &gap_leaf, &index_bits, siblings)?;

    // Conditional constraint: if real (not dummy), computed root must match ofac_root
    let root_diff = ofac_root.clone() - computed_root;
    (root_diff * is_real_fp.clone()).enforce_equal(&zero)?;

    // 5. VALUE ACCUMULATION
    //    Real step: add value to total
    //    Dummy step: subtract value from total (for batch padding, net zero effect when value=0)
    let two = F::from(2u64);
    let factor = one - is_dummy_fp * FpVar::<F>::constant(two);
    let new_total_value = prev_total_value.clone() + value.clone() * factor;
    enforce_bit_length(&new_total_value, BYTES31_BIT_LENGTH)?;

    Ok(new_total_value)
}

#[cfg(test)]
mod tests {
    use super::innocence_step;
    use crate::{
        circuits::burn_address::{
            compute_burn_address_from_secret, find_pow_nonce, secret_from_nonce,
        },
        utils::poseidon::{
            gadgets::CircomCRHParametersVar,
            utils::{circom_poseidon2_config, circom_poseidon3_config, poseidon2},
        },
    };
    use ark_bn254::Fr;
    use ark_r1cs_std::{alloc::AllocVar, boolean::Boolean, eq::EqGadget, fields::fp::FpVar};
    use ark_relations::{
        gr1cs::{ConstraintSystem, SynthesisError},
        ns,
    };

    const DEPTH: usize = 4;

    /// Helper to build a simple exclusion tree for testing.
    fn build_test_exclusion_tree(gaps: &[(Fr, Fr)]) -> Fr {
        let leaves: Vec<Fr> = gaps.iter().map(|(s, e)| poseidon2(*s, *e)).collect();

        let mut padded_leaves = leaves.clone();
        while padded_leaves.len() < (1 << DEPTH) {
            padded_leaves.push(Fr::from(0u64));
        }

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

    /// Helper to find a valid secret for a recipient (satisfies PoW)
    fn find_valid_secret(recipient: Fr, seed: u64) -> Fr {
        let secret_seed = Fr::from(seed);
        let nonce = find_pow_nonce(recipient, secret_seed);
        secret_from_nonce(secret_seed, nonce)
    }

    #[test]
    fn innocence_step_accepts_valid_proof_with_recipient_binding() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon3_config = circom_poseidon3_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;
        let poseidon3_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon3_params"), &poseidon3_config)?;

        // Setup exclusion tree
        let gaps = vec![
            (Fr::from(0u64), Fr::from(100u64)),
            (Fr::from(100u64), Fr::from(200u64)),
            (Fr::from(200u64), Fr::from(1000u64)),
        ];
        let ofac_root_value = build_test_exclusion_tree(&gaps);

        // Setup recipient and valid secret (satisfies PoW)
        let recipient_value = Fr::from(12345u64);
        let secret_value = find_valid_secret(recipient_value, 1000);

        // Verify secret satisfies PoW
        compute_burn_address_from_secret(recipient_value, secret_value)
            .expect("secret should satisfy PoW");

        // Test: prove address 150 is not sanctioned
        let from_address_value = Fr::from(150u64);
        let start_value = Fr::from(100u64);
        let end_value = Fr::from(200u64);
        let gap_index_value = Fr::from(1u64);
        let value_value = Fr::from(1000u64);
        let prev_total_value = Fr::from(500u64);
        let is_dummy_value = false;

        let siblings_values = get_merkle_proof(&gaps, 1);

        // Allocate variables
        let ofac_root = FpVar::<Fr>::new_witness(ns!(cs, "ofac_root"), || Ok(ofac_root_value))?;
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let from_address =
            FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_address_value))?;
        let prev_total = FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(prev_total_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(is_dummy_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(start_value))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(end_value))?;
        let gap_index = FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || Ok(gap_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let new_total = innocence_step::<Fr, DEPTH>(
            &poseidon2_params,
            &poseidon3_params,
            &ofac_root,
            &recipient,
            &from_address,
            &prev_total,
            &is_dummy,
            &value,
            &secret,
            &start,
            &end,
            &gap_index,
            &siblings,
        )?;

        let expected_total = Fr::from(1500u64);
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_total))?;
        new_total.enforce_equal(&expected_var)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn innocence_step_rejects_wrong_recipient() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon3_config = circom_poseidon3_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;
        let poseidon3_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon3_params"), &poseidon3_config)?;

        let gaps = vec![
            (Fr::from(0u64), Fr::from(100u64)),
            (Fr::from(100u64), Fr::from(200u64)),
            (Fr::from(200u64), Fr::from(1000u64)),
        ];
        let ofac_root_value = build_test_exclusion_tree(&gaps);

        // Alice's recipient and valid secret
        let alice_recipient = Fr::from(12345u64);
        let alice_secret = find_valid_secret(alice_recipient, 1000);

        // ATTACK: Carol tries to use Alice's secret with her own recipient
        let carol_recipient = Fr::from(99999u64);

        let from_address_value = Fr::from(150u64);
        let start_value = Fr::from(100u64);
        let end_value = Fr::from(200u64);
        let gap_index_value = Fr::from(1u64);
        let value_value = Fr::from(1000u64);
        let prev_total_value = Fr::from(0u64);
        let is_dummy_value = false;

        let siblings_values = get_merkle_proof(&gaps, 1);

        let ofac_root = FpVar::<Fr>::new_witness(ns!(cs, "ofac_root"), || Ok(ofac_root_value))?;
        // Carol's recipient but Alice's secret
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(carol_recipient))?;
        let from_address =
            FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_address_value))?;
        let prev_total = FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(prev_total_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(is_dummy_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(alice_secret))?;
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(start_value))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(end_value))?;
        let gap_index = FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || Ok(gap_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let new_total = innocence_step::<Fr, DEPTH>(
            &poseidon2_params,
            &poseidon3_params,
            &ofac_root,
            &recipient,
            &from_address,
            &prev_total,
            &is_dummy,
            &value,
            &secret,
            &start,
            &end,
            &gap_index,
            &siblings,
        )?;

        let expected_total = Fr::from(1000u64);
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_total))?;
        new_total.enforce_equal(&expected_var)?;

        // Should NOT be satisfied - PoW fails for (carol_recipient, alice_secret)
        assert!(!cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn innocence_step_rejects_sanctioned_address() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon3_config = circom_poseidon3_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;
        let poseidon3_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon3_params"), &poseidon3_config)?;

        let gaps = vec![
            (Fr::from(0u64), Fr::from(100u64)),
            (Fr::from(100u64), Fr::from(200u64)),
            (Fr::from(200u64), Fr::from(1000u64)),
        ];
        let ofac_root_value = build_test_exclusion_tree(&gaps);

        let recipient_value = Fr::from(12345u64);
        let secret_value = find_valid_secret(recipient_value, 2000);

        // Try to prove address 100 is not sanctioned (but it's at the boundary = sanctioned)
        let from_address_value = Fr::from(100u64);
        let start_value = Fr::from(100u64);
        let end_value = Fr::from(200u64);
        let gap_index_value = Fr::from(1u64);
        let value_value = Fr::from(1000u64);
        let prev_total_value = Fr::from(0u64);
        let is_dummy_value = false;

        let siblings_values = get_merkle_proof(&gaps, 1);

        let ofac_root = FpVar::<Fr>::new_witness(ns!(cs, "ofac_root"), || Ok(ofac_root_value))?;
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let from_address =
            FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_address_value))?;
        let prev_total = FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(prev_total_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(is_dummy_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(start_value))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(end_value))?;
        let gap_index = FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || Ok(gap_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let new_total = innocence_step::<Fr, DEPTH>(
            &poseidon2_params,
            &poseidon3_params,
            &ofac_root,
            &recipient,
            &from_address,
            &prev_total,
            &is_dummy,
            &value,
            &secret,
            &start,
            &end,
            &gap_index,
            &siblings,
        )?;

        let expected_total = Fr::from(1000u64);
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_total))?;
        new_total.enforce_equal(&expected_var)?;

        // Should NOT be satisfied - from_address == start (not strictly greater)
        assert!(!cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn innocence_step_dummy_skips_all_verification() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon3_config = circom_poseidon3_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;
        let poseidon3_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon3_params"), &poseidon3_config)?;

        // For dummy steps, we can use invalid data - it should still satisfy
        let ofac_root_value = Fr::from(999999u64);
        let recipient_value = Fr::from(11111u64);
        let secret_value = Fr::from(22222u64); // Invalid secret (won't pass PoW)
        let from_address_value = Fr::from(100u64);
        let start_value = Fr::from(100u64); // Invalid: from == start
        let end_value = Fr::from(200u64);
        let gap_index_value = Fr::from(0u64);
        let value_value = Fr::from(50u64);
        let prev_total_value = Fr::from(100u64);
        let is_dummy_value = true; // DUMMY - skip ALL verification

        let siblings_values = vec![Fr::from(0u64); DEPTH];

        let ofac_root = FpVar::<Fr>::new_witness(ns!(cs, "ofac_root"), || Ok(ofac_root_value))?;
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let from_address =
            FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_address_value))?;
        let prev_total = FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(prev_total_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(is_dummy_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(start_value))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(end_value))?;
        let gap_index = FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || Ok(gap_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let new_total = innocence_step::<Fr, DEPTH>(
            &poseidon2_params,
            &poseidon3_params,
            &ofac_root,
            &recipient,
            &from_address,
            &prev_total,
            &is_dummy,
            &value,
            &secret,
            &start,
            &end,
            &gap_index,
            &siblings,
        )?;

        // Dummy subtracts: prev_total - value = 100 - 50 = 50
        let expected_total = Fr::from(50u64);
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_total))?;
        new_total.enforce_equal(&expected_var)?;

        // Should be satisfied - is_dummy=true skips ALL verification
        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn innocence_step_rejects_wrong_merkle_proof() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon3_config = circom_poseidon3_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;
        let poseidon3_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon3_params"), &poseidon3_config)?;

        let gaps = vec![
            (Fr::from(0u64), Fr::from(100u64)),
            (Fr::from(100u64), Fr::from(200u64)),
            (Fr::from(200u64), Fr::from(1000u64)),
        ];
        let ofac_root_value = build_test_exclusion_tree(&gaps);

        let recipient_value = Fr::from(12345u64);
        let secret_value = find_valid_secret(recipient_value, 3000);

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
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let from_address =
            FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_address_value))?;
        let prev_total = FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(prev_total_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(is_dummy_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(start_value))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(end_value))?;
        let gap_index = FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || Ok(gap_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let new_total = innocence_step::<Fr, DEPTH>(
            &poseidon2_params,
            &poseidon3_params,
            &ofac_root,
            &recipient,
            &from_address,
            &prev_total,
            &is_dummy,
            &value,
            &secret,
            &start,
            &end,
            &gap_index,
            &siblings,
        )?;

        let expected_total = Fr::from(1000u64);
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_total))?;
        new_total.enforce_equal(&expected_var)?;

        // Should NOT be satisfied - Merkle proof doesn't match
        assert!(!cs.is_satisfied().unwrap());
        Ok(())
    }
}
