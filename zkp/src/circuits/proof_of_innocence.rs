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
        tree::gadgets::{
            leaf_hash::leaf_hash_var,
            merkle::{
                enforce_bit_length, enforce_strict_less_than, merkle_root_from_leaf,
                to_bits_le_limited,
            },
        },
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

type InnocenceStepResult<F> = Result<(FpVar<F>, FpVar<F>), SynthesisError>;

/// Single step of the Proof of Innocence Nova circuit.
///
/// For each transfer to a recipient, proves:
/// 1. **Recipient binding**: The transfer belongs to this recipient (via burn address PoW)
/// 2. **Non-membership**: The sender (`from_address`) is NOT in the OFAC sanctions list
/// 3. **Transfer inclusion**: The transfer exists in the transfer tree (Merkle proof)
/// 4. **No replay**: Leaf indices are strictly monotonically increasing
///
/// # Arguments
///
/// * `poseidon2_params` - Poseidon config for 2-to-1 hashing (gap leaf and Merkle tree)
/// * `poseidon3_params` - Poseidon config for 3-to-1 hashing (burn address derivation and leaf hash)
/// * `ofac_root` - Root of the OFAC exclusion tree (public, constant across steps)
/// * `recipient` - Hash of GeneralRecipient (public, constant across steps)
/// * `merkle_root` - Root of the transfer tree (public, constant across steps)
/// * `from_address` - Sender address to prove is not sanctioned
/// * `prev_leaf_index_with_offset` - Previous step's leaf_index + 1 (for monotonicity)
/// * `prev_total_value` - Running sum of transfer values from previous steps
/// * `is_dummy` - Whether this is a padding step (skips verification, subtracts value)
/// * `value` - This transfer's value
/// * `secret` - Secret used to derive the burn address (proves transfer belongs to recipient)
/// * `leaf_index` - Position of this transfer in the transfer tree
/// * `transfer_siblings` - Merkle proof siblings for the transfer leaf
/// * `start` - Lower bound of the exclusion gap containing `from_address`
/// * `end` - Upper bound of the exclusion gap containing `from_address`
/// * `gap_index` - Position of this gap leaf in the exclusion tree
/// * `siblings` - Merkle proof siblings for the gap leaf
///
/// # Returns
///
/// `(leaf_index_with_offset, new_total_value)` — the updated leaf index tracker and accumulated total.
///
/// # Constraints
///
/// 1. **Recipient binding**: `burn_address = derive(recipient, secret)` with PoW check
/// 2. Range checks on all inputs
/// 3. Exclusion proof: `start < from_address < end`
/// 4. Gap leaf hash: `poseidon2(start, end)`
/// 5. Merkle proof verification against `ofac_root`
/// 6. **Transfer tree inclusion**: leaf hash verified against `merkle_root`
/// 7. **Monotonicity**: `prev_leaf_index_with_offset < leaf_index + 1`
/// 8. Value accumulation: `new_total = prev_total + value` (or `- value` for dummy)
#[allow(clippy::too_many_arguments)]
pub fn innocence_step<F, const DEPTH: usize, const TRANSFER_DEPTH: usize>(
    poseidon2_params: &CircomCRHParametersVar<F>,
    poseidon3_params: &CircomCRHParametersVar<F>,
    ofac_root: &FpVar<F>,
    recipient: &FpVar<F>,
    merkle_root: &FpVar<F>,
    from_address: &FpVar<F>,
    prev_leaf_index_with_offset: &FpVar<F>,
    prev_total_value: &FpVar<F>,
    is_dummy: &Boolean<F>,
    value: &FpVar<F>,
    secret: &FpVar<F>,
    leaf_index: &FpVar<F>,
    transfer_siblings: &[FpVar<F>],
    start: &FpVar<F>,
    end: &FpVar<F>,
    gap_index: &FpVar<F>,
    siblings: &[FpVar<F>],
) -> InnocenceStepResult<F>
where
    F: PrimeField + Absorb,
{
    assert_eq!(siblings.len(), DEPTH);
    assert_eq!(transfer_siblings.len(), TRANSFER_DEPTH);

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
    //    Burn address is also used for the transfer tree leaf hash computation.
    let burn_address = burn_address_var(poseidon3_params, recipient, secret, &should_constrain)?;

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

    // 5. TRANSFER TREE INCLUSION PROOF
    //    Verify the transfer exists in the transfer tree and enforce monotonically
    //    increasing leaf indices to prevent replay attacks.

    // Range checks for transfer tree indices
    enforce_bit_length(leaf_index, TRANSFER_DEPTH)?;
    enforce_bit_length(prev_leaf_index_with_offset, TRANSFER_DEPTH + 1)?;

    // Monotonicity: prev_leaf_index_with_offset < leaf_index + 1
    let leaf_index_with_offset = leaf_index.clone() + one.clone();
    enforce_strict_less_than(
        prev_leaf_index_with_offset,
        &leaf_index_with_offset,
        TRANSFER_DEPTH + 1,
    )?;

    // Compute leaf hash: poseidon3(from_address, burn_address, value)
    let leaf_hash = leaf_hash_var(poseidon3_params, from_address, &burn_address, value)?;

    // Verify Merkle proof against merkle_root (conditional on is_real)
    let transfer_index_bits = to_bits_le_limited(leaf_index, TRANSFER_DEPTH)?;
    let computed_transfer_root = merkle_root_from_leaf(
        poseidon2_params,
        &leaf_hash,
        &transfer_index_bits,
        transfer_siblings,
    )?;
    let transfer_root_diff = merkle_root.clone() - computed_transfer_root;
    (transfer_root_diff * is_real_fp.clone()).enforce_equal(&zero)?;

    // 6. VALUE ACCUMULATION
    //    Real step: add value to total
    //    Dummy step: subtract value from total (for batch padding, net zero effect when value=0)
    let two = F::from(2u64);
    let factor = one - is_dummy_fp * FpVar::<F>::constant(two);
    let new_total_value = prev_total_value.clone() + value.clone() * factor;
    enforce_bit_length(&new_total_value, BYTES31_BIT_LENGTH)?;

    Ok((leaf_index_with_offset, new_total_value))
}

#[cfg(test)]
mod tests {
    use super::innocence_step;
    use crate::{
        circuits::burn_address::{
            compute_burn_address_from_secret, find_pow_nonce, secret_from_nonce,
        },
        utils::{
            convertion::{fr_to_address, fr_to_u256},
            exclusion_tree::ExclusionTree,
            poseidon::{
                gadgets::CircomCRHParametersVar,
                utils::{circom_poseidon2_config, circom_poseidon3_config},
            },
            tree::incremental_merkle_tree::IncrementalMerkleTree,
        },
    };
    use ark_bn254::Fr;
    use ark_r1cs_std::{alloc::AllocVar, boolean::Boolean, eq::EqGadget, fields::fp::FpVar};
    use ark_relations::{
        gr1cs::{ConstraintSystem, SynthesisError},
        ns,
    };

    const OFAC_LIST: &str = include_str!("../../data/ofac_sanction_list.txt");
    // 81 sanctioned addresses → 82 gaps → need 2^7 = 128 leaves
    const DEPTH: usize = 7;
    const TRANSFER_DEPTH: usize = 4;

    fn build_ofac_tree() -> ExclusionTree {
        let lines: Vec<&str> = OFAC_LIST.lines().collect();
        let addresses = ExclusionTree::parse_addresses(&lines);
        ExclusionTree::from_sorted_addresses(&addresses, DEPTH)
    }

    /// Helper to find a valid secret for a recipient (satisfies PoW)
    fn find_valid_secret(recipient: Fr, seed: u64) -> Fr {
        let secret_seed = Fr::from(seed);
        let nonce = find_pow_nonce(recipient, secret_seed);
        secret_from_nonce(secret_seed, nonce)
    }

    /// Build a transfer tree and insert a leaf for the given transfer.
    /// Returns (transfer_tree_root, leaf_index, transfer_siblings).
    fn build_transfer_tree(
        from_address_value: Fr,
        burn_address_value: Fr,
        value_value: Fr,
    ) -> (Fr, u64, [Fr; TRANSFER_DEPTH]) {
        let mut transfer_tree = IncrementalMerkleTree::new(TRANSFER_DEPTH);
        let from_addr = fr_to_address(from_address_value);
        let burn_addr = fr_to_address(burn_address_value);
        let value_u256 = fr_to_u256(value_value);
        let leaf_index = transfer_tree
            .insert(from_addr, burn_addr, value_u256)
            .expect("insert should succeed");
        let transfer_root = transfer_tree.get_root();
        let transfer_proof = transfer_tree.prove(leaf_index);
        let transfer_siblings: [Fr; TRANSFER_DEPTH] = transfer_proof.siblings.try_into().unwrap();
        (transfer_root, leaf_index, transfer_siblings)
    }

    // First sanctioned address on the list
    const FIRST_SANCTIONED: &str = "0x04dba1194ee10112fe6c3207c0687def0e78bacf";
    // An innocent address that falls in the gap between the 1st and 2nd sanctioned addresses
    const INNOCENT_ADDRESS: &str = "0x0600000000000000000000000000000000000000";

    #[test]
    fn innocence_step_accepts_valid_proof_with_recipient_binding() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon3_config = circom_poseidon3_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;
        let poseidon3_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon3_params"), &poseidon3_config)?;

        let tree = build_ofac_tree();
        let ofac_root_value = tree.root();

        // Setup recipient and valid secret (satisfies PoW)
        let recipient_value = Fr::from(12345u64);
        let secret_value = find_valid_secret(recipient_value, 1000);

        // Verify secret satisfies PoW
        let burn_address_value = compute_burn_address_from_secret(recipient_value, secret_value)
            .expect("secret should satisfy PoW");

        // Prove an innocent address is not sanctioned
        let from_address_value = ExclusionTree::parse_addresses(&[INNOCENT_ADDRESS])[0];
        let proof = tree
            .prove_non_membership(from_address_value)
            .expect("address should not be sanctioned");

        let value_value = Fr::from(1000u64);
        let prev_total_value = Fr::from(500u64);

        // Build transfer tree with this transfer
        let (transfer_root_value, leaf_index_u64, transfer_siblings_values) =
            build_transfer_tree(from_address_value, burn_address_value, value_value);

        // Allocate variables
        let ofac_root = FpVar::<Fr>::new_witness(ns!(cs, "ofac_root"), || Ok(ofac_root_value))?;
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let transfer_root =
            FpVar::<Fr>::new_witness(ns!(cs, "transfer_root"), || Ok(transfer_root_value))?;
        let from_address =
            FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_address_value))?;
        let prev_leaf_index_with_offset =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_leaf_idx"), || Ok(Fr::from(0u64)))?;
        let prev_total = FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(prev_total_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(false))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let leaf_index =
            FpVar::<Fr>::new_witness(ns!(cs, "leaf_index"), || Ok(Fr::from(leaf_index_u64)))?;
        let transfer_siblings = transfer_siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "transfer_sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(proof.start))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(proof.end))?;
        let gap_index =
            FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || Ok(Fr::from(proof.gap_index)))?;
        let siblings = proof
            .siblings
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let (out_leaf_idx, new_total) = innocence_step::<Fr, DEPTH, TRANSFER_DEPTH>(
            &poseidon2_params,
            &poseidon3_params,
            &ofac_root,
            &recipient,
            &transfer_root,
            &from_address,
            &prev_leaf_index_with_offset,
            &prev_total,
            &is_dummy,
            &value,
            &secret,
            &leaf_index,
            &transfer_siblings,
            &start,
            &end,
            &gap_index,
            &siblings,
        )?;

        let expected_total = Fr::from(1500u64);
        let expected_total_var =
            FpVar::<Fr>::new_input(ns!(cs, "expected_total"), || Ok(expected_total))?;
        new_total.enforce_equal(&expected_total_var)?;

        let expected_leaf_idx = Fr::from(leaf_index_u64 + 1);
        let expected_leaf_idx_var =
            FpVar::<Fr>::new_input(ns!(cs, "expected_leaf_idx"), || Ok(expected_leaf_idx))?;
        out_leaf_idx.enforce_equal(&expected_leaf_idx_var)?;

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

        let tree = build_ofac_tree();
        let ofac_root_value = tree.root();

        // Alice's recipient and valid secret
        let alice_recipient = Fr::from(12345u64);
        let alice_secret = find_valid_secret(alice_recipient, 1000);
        let alice_burn =
            compute_burn_address_from_secret(alice_recipient, alice_secret).expect("valid PoW");

        // ATTACK: Carol tries to use Alice's secret with her own recipient
        let carol_recipient = Fr::from(99999u64);

        let from_address_value = ExclusionTree::parse_addresses(&[INNOCENT_ADDRESS])[0];
        let proof = tree
            .prove_non_membership(from_address_value)
            .expect("address should not be sanctioned");

        let value_value = Fr::from(1000u64);
        let prev_total_value = Fr::from(0u64);

        // Build transfer tree using Alice's burn address
        let (transfer_root_value, leaf_index_u64, transfer_siblings_values) =
            build_transfer_tree(from_address_value, alice_burn, value_value);

        let ofac_root = FpVar::<Fr>::new_witness(ns!(cs, "ofac_root"), || Ok(ofac_root_value))?;
        // Carol's recipient but Alice's secret
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(carol_recipient))?;
        let transfer_root =
            FpVar::<Fr>::new_witness(ns!(cs, "transfer_root"), || Ok(transfer_root_value))?;
        let from_address =
            FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_address_value))?;
        let prev_leaf_index_with_offset =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_leaf_idx"), || Ok(Fr::from(0u64)))?;
        let prev_total = FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(prev_total_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(false))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(alice_secret))?;
        let leaf_index =
            FpVar::<Fr>::new_witness(ns!(cs, "leaf_index"), || Ok(Fr::from(leaf_index_u64)))?;
        let transfer_siblings = transfer_siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "transfer_sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(proof.start))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(proof.end))?;
        let gap_index =
            FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || Ok(Fr::from(proof.gap_index)))?;
        let siblings = proof
            .siblings
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let (_out_leaf_idx, new_total) = innocence_step::<Fr, DEPTH, TRANSFER_DEPTH>(
            &poseidon2_params,
            &poseidon3_params,
            &ofac_root,
            &recipient,
            &transfer_root,
            &from_address,
            &prev_leaf_index_with_offset,
            &prev_total,
            &is_dummy,
            &value,
            &secret,
            &leaf_index,
            &transfer_siblings,
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

        let tree = build_ofac_tree();
        let ofac_root_value = tree.root();

        let recipient_value = Fr::from(12345u64);
        let secret_value = find_valid_secret(recipient_value, 2000);
        let burn_address_value =
            compute_burn_address_from_secret(recipient_value, secret_value).expect("valid PoW");

        // The sanctioned address itself — prove_non_membership returns None
        let sanctioned = ExclusionTree::parse_addresses(&[FIRST_SANCTIONED])[0];
        assert!(tree.prove_non_membership(sanctioned).is_none());

        // Use the neighbouring gap's proof but with the sanctioned address as from_address.
        let innocent_neighbor = ExclusionTree::parse_addresses(&[INNOCENT_ADDRESS])[0];
        let proof = tree
            .prove_non_membership(innocent_neighbor)
            .expect("neighbor should not be sanctioned");

        let value_value = Fr::from(1000u64);
        let prev_total_value = Fr::from(0u64);

        // Build transfer tree (using sanctioned from_address for the leaf)
        let (transfer_root_value, leaf_index_u64, transfer_siblings_values) =
            build_transfer_tree(sanctioned, burn_address_value, value_value);

        let ofac_root = FpVar::<Fr>::new_witness(ns!(cs, "ofac_root"), || Ok(ofac_root_value))?;
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let transfer_root =
            FpVar::<Fr>::new_witness(ns!(cs, "transfer_root"), || Ok(transfer_root_value))?;
        // Sanctioned address as from_address
        let from_address = FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(sanctioned))?;
        let prev_leaf_index_with_offset =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_leaf_idx"), || Ok(Fr::from(0u64)))?;
        let prev_total = FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(prev_total_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(false))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let leaf_index =
            FpVar::<Fr>::new_witness(ns!(cs, "leaf_index"), || Ok(Fr::from(leaf_index_u64)))?;
        let transfer_siblings = transfer_siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "transfer_sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(proof.start))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(proof.end))?;
        let gap_index =
            FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || Ok(Fr::from(proof.gap_index)))?;
        let siblings = proof
            .siblings
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let (_out_leaf_idx, new_total) = innocence_step::<Fr, DEPTH, TRANSFER_DEPTH>(
            &poseidon2_params,
            &poseidon3_params,
            &ofac_root,
            &recipient,
            &transfer_root,
            &from_address,
            &prev_leaf_index_with_offset,
            &prev_total,
            &is_dummy,
            &value,
            &secret,
            &leaf_index,
            &transfer_siblings,
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
        let transfer_root_value = Fr::from(888888u64);
        let secret_value = Fr::from(22222u64); // Invalid secret (won't pass PoW)
        let from_address_value = Fr::from(100u64);
        let start_value = Fr::from(100u64); // Invalid: from == start
        let end_value = Fr::from(200u64);
        let gap_index_value = Fr::from(0u64);
        let value_value = Fr::from(50u64);
        let prev_total_value = Fr::from(100u64);
        let prev_leaf_index_with_offset_value = Fr::from(2u64);
        let leaf_index_value = Fr::from(2u64);

        let siblings_values = vec![Fr::from(0u64); DEPTH];
        let transfer_siblings_values = [Fr::from(0u64); TRANSFER_DEPTH];

        let ofac_root = FpVar::<Fr>::new_witness(ns!(cs, "ofac_root"), || Ok(ofac_root_value))?;
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let transfer_root =
            FpVar::<Fr>::new_witness(ns!(cs, "transfer_root"), || Ok(transfer_root_value))?;
        let from_address =
            FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_address_value))?;
        let prev_leaf_index_with_offset =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_leaf_idx"), || {
                Ok(prev_leaf_index_with_offset_value)
            })?;
        let prev_total = FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(prev_total_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(true))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let leaf_index = FpVar::<Fr>::new_witness(ns!(cs, "leaf_index"), || Ok(leaf_index_value))?;
        let transfer_siblings = transfer_siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "transfer_sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(start_value))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(end_value))?;
        let gap_index = FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || Ok(gap_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let (out_leaf_idx, new_total) = innocence_step::<Fr, DEPTH, TRANSFER_DEPTH>(
            &poseidon2_params,
            &poseidon3_params,
            &ofac_root,
            &recipient,
            &transfer_root,
            &from_address,
            &prev_leaf_index_with_offset,
            &prev_total,
            &is_dummy,
            &value,
            &secret,
            &leaf_index,
            &transfer_siblings,
            &start,
            &end,
            &gap_index,
            &siblings,
        )?;

        // Dummy subtracts: prev_total - value = 100 - 50 = 50
        let expected_total = Fr::from(50u64);
        let expected_total_var =
            FpVar::<Fr>::new_input(ns!(cs, "expected_total"), || Ok(expected_total))?;
        new_total.enforce_equal(&expected_total_var)?;

        // leaf_index_with_offset = leaf_index + 1 = 3
        let expected_leaf_idx = Fr::from(3u64);
        let expected_leaf_idx_var =
            FpVar::<Fr>::new_input(ns!(cs, "expected_leaf_idx"), || Ok(expected_leaf_idx))?;
        out_leaf_idx.enforce_equal(&expected_leaf_idx_var)?;

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

        let tree = build_ofac_tree();
        let ofac_root_value = tree.root();

        let recipient_value = Fr::from(12345u64);
        let secret_value = find_valid_secret(recipient_value, 3000);
        let burn_address_value =
            compute_burn_address_from_secret(recipient_value, secret_value).expect("valid PoW");

        // Get a valid proof for our innocent address
        let from_address_value = ExclusionTree::parse_addresses(&[INNOCENT_ADDRESS])[0];
        let correct_proof = tree
            .prove_non_membership(from_address_value)
            .expect("address should not be sanctioned");

        // Get a WRONG proof from a different gap (gap 0, which is before the first sanctioned addr)
        let wrong_proof = tree
            .prove_non_membership(Fr::from(1u64))
            .expect("address 1 should not be sanctioned");

        let value_value = Fr::from(1000u64);
        let prev_total_value = Fr::from(0u64);

        // Build transfer tree
        let (transfer_root_value, leaf_index_u64, transfer_siblings_values) =
            build_transfer_tree(from_address_value, burn_address_value, value_value);

        let ofac_root = FpVar::<Fr>::new_witness(ns!(cs, "ofac_root"), || Ok(ofac_root_value))?;
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let transfer_root =
            FpVar::<Fr>::new_witness(ns!(cs, "transfer_root"), || Ok(transfer_root_value))?;
        let from_address =
            FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_address_value))?;
        let prev_leaf_index_with_offset =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_leaf_idx"), || Ok(Fr::from(0u64)))?;
        let prev_total = FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(prev_total_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(false))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let leaf_index =
            FpVar::<Fr>::new_witness(ns!(cs, "leaf_index"), || Ok(Fr::from(leaf_index_u64)))?;
        let transfer_siblings = transfer_siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "transfer_sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;
        // Use correct gap boundaries but wrong Merkle siblings
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(correct_proof.start))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(correct_proof.end))?;
        let gap_index = FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || {
            Ok(Fr::from(correct_proof.gap_index))
        })?;
        // Wrong siblings from a different gap's proof
        let siblings = wrong_proof
            .siblings
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let (_out_leaf_idx, new_total) = innocence_step::<Fr, DEPTH, TRANSFER_DEPTH>(
            &poseidon2_params,
            &poseidon3_params,
            &ofac_root,
            &recipient,
            &transfer_root,
            &from_address,
            &prev_leaf_index_with_offset,
            &prev_total,
            &is_dummy,
            &value,
            &secret,
            &leaf_index,
            &transfer_siblings,
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

    #[test]
    fn innocence_step_rejects_replay_same_leaf_index() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon3_config = circom_poseidon3_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;
        let poseidon3_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon3_params"), &poseidon3_config)?;

        let tree = build_ofac_tree();
        let ofac_root_value = tree.root();

        let recipient_value = Fr::from(12345u64);
        let secret_value = find_valid_secret(recipient_value, 4000);
        let burn_address_value =
            compute_burn_address_from_secret(recipient_value, secret_value).expect("valid PoW");

        let from_address_value = ExclusionTree::parse_addresses(&[INNOCENT_ADDRESS])[0];
        let proof = tree
            .prove_non_membership(from_address_value)
            .expect("address should not be sanctioned");

        let value_value = Fr::from(1000u64);

        // Build transfer tree
        let (transfer_root_value, leaf_index_u64, transfer_siblings_values) =
            build_transfer_tree(from_address_value, burn_address_value, value_value);

        // ATTACK: Replay the same leaf_index.
        // Simulate that the first step already consumed this leaf (prev_leaf_index_with_offset = leaf_index + 1).
        let prev_leaf_index_with_offset_value = Fr::from(leaf_index_u64 + 1);

        let ofac_root = FpVar::<Fr>::new_witness(ns!(cs, "ofac_root"), || Ok(ofac_root_value))?;
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let transfer_root =
            FpVar::<Fr>::new_witness(ns!(cs, "transfer_root"), || Ok(transfer_root_value))?;
        let from_address =
            FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_address_value))?;
        let prev_leaf_index_with_offset =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_leaf_idx"), || {
                Ok(prev_leaf_index_with_offset_value)
            })?;
        let prev_total = FpVar::<Fr>::new_witness(ns!(cs, "prev_total"), || Ok(Fr::from(1000u64)))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(false))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let leaf_index =
            FpVar::<Fr>::new_witness(ns!(cs, "leaf_index"), || Ok(Fr::from(leaf_index_u64)))?;
        let transfer_siblings = transfer_siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "transfer_sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;
        let start = FpVar::<Fr>::new_witness(ns!(cs, "start"), || Ok(proof.start))?;
        let end = FpVar::<Fr>::new_witness(ns!(cs, "end"), || Ok(proof.end))?;
        let gap_index =
            FpVar::<Fr>::new_witness(ns!(cs, "gap_index"), || Ok(Fr::from(proof.gap_index)))?;
        let siblings = proof
            .siblings
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let (_out_leaf_idx, new_total) = innocence_step::<Fr, DEPTH, TRANSFER_DEPTH>(
            &poseidon2_params,
            &poseidon3_params,
            &ofac_root,
            &recipient,
            &transfer_root,
            &from_address,
            &prev_leaf_index_with_offset,
            &prev_total,
            &is_dummy,
            &value,
            &secret,
            &leaf_index,
            &transfer_siblings,
            &start,
            &end,
            &gap_index,
            &siblings,
        )?;

        let expected_total = Fr::from(2000u64);
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_total))?;
        new_total.enforce_equal(&expected_var)?;

        // Should NOT be satisfied - monotonicity violated (replayed leaf_index)
        assert!(!cs.is_satisfied().unwrap());
        Ok(())
    }
}
