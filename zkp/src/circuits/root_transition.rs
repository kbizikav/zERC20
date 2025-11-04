use crate::circuits::constants::{ADDRESS_BIT_LENGTH, BYTES31_BIT_LENGTH};
use crate::utils::poseidon::gadgets::CircomCRHParametersVar;
use crate::utils::tree::gadgets::{
    hash_chain::hash_chain_var,
    leaf_hash::leaf_hash_var,
    merkle::{enforce_bit_length, merkle_root_from_leaf, to_bits_le_limited},
};
use ark_crypto_primitives::sponge::Absorb;
use ark_ff::PrimeField;
use ark_r1cs_std::{
    boolean::Boolean,
    eq::EqGadget,
    fields::{FieldVar, fp::FpVar},
    select::CondSelectGadget,
};
use ark_relations::gr1cs::SynthesisError;
use core::ops::Not;

pub fn root_transition_step<F, const DEPTH: usize>(
    poseidon_params: &CircomCRHParametersVar<F>,
    index: &FpVar<F>,
    prev_hash_chain: &FpVar<F>,
    prev_root: &FpVar<F>,
    address: &FpVar<F>,
    value: &FpVar<F>,
    siblings: &[FpVar<F>],
    is_dummy: &Boolean<F>,
) -> Result<(FpVar<F>, FpVar<F>, FpVar<F>), SynthesisError>
where
    F: PrimeField + Absorb,
{
    assert_eq!(siblings.len(), DEPTH);

    // range check inputs
    enforce_bit_length(address, ADDRESS_BIT_LENGTH)?;
    enforce_bit_length(value, BYTES31_BIT_LENGTH)?;

    // index is range-checked here
    let index_bits = to_bits_le_limited(index, DEPTH)?;

    let zero_leaf = FpVar::<F>::constant(F::zero());
    let prev_merkle = merkle_root_from_leaf(poseidon_params, &zero_leaf, &index_bits, siblings)?;
    let should_enforce = is_dummy.clone().not();
    prev_root.conditional_enforce_equal(&prev_merkle, &should_enforce)?;

    let leaf_hash = leaf_hash_var(poseidon_params, address, value)?;
    let new_root = merkle_root_from_leaf(poseidon_params, &leaf_hash, &index_bits, siblings)?;

    let new_hash_chain = hash_chain_var(prev_hash_chain, address, value)?;
    let new_index_candidate = index.clone() + FpVar::<F>::constant(F::one());

    let new_index = FpVar::<F>::conditionally_select(is_dummy, index, &new_index_candidate)?;
    let new_hash_chain =
        FpVar::<F>::conditionally_select(is_dummy, prev_hash_chain, &new_hash_chain)?;
    let new_root = FpVar::<F>::conditionally_select(is_dummy, prev_root, &new_root)?;

    Ok((new_index, new_hash_chain, new_root))
}

#[cfg(test)]
mod tests {
    use super::root_transition_step;
    use crate::test_utils::merkle_root_from_path;
    use crate::utils::poseidon::gadgets::CircomCRHParametersVar;
    use crate::utils::poseidon::utils::{circom_poseidon_config, circom_poseidon_hash};
    use ark_bn254::Fr;
    use ark_ff::{PrimeField, Zero};
    use ark_r1cs_std::{alloc::AllocVar, boolean::Boolean, eq::EqGadget, fields::fp::FpVar};
    use ark_relations::gr1cs::{ConstraintSystem, SynthesisError};
    use ark_relations::ns;
    use ark_std::vec::Vec;
    use sha2::{Digest, Sha256};

    const DEPTH: usize = 4;
    const ROOT_TRANSITION_NEW_HASH_CHAIN_HEX: &str =
        "0x843bb3a68dcad81b8a130f0eae7be44d20354e572b4f6ad726b701095b4bc2";
    const ROOT_TRANSITION_LEAF_HASH_HEX: &str =
        "0x0f84d7bd85ce2c2ce883ff9db998edb4d033663faee4082a46700144d631d1a3";
    const ROOT_TRANSITION_OLD_ROOT_HEX: &str =
        "0x161b3b682780534f65ad950d76def4b011da86a2e1c71297d8dbd62a394cc8fb";
    const ROOT_TRANSITION_NEW_ROOT_HEX: &str =
        "0x2aa763593530d46e14f5af0f8a265103ee5890eaef5c09910dc3318b1224650e";

    #[test]
    fn root_transition_matches_reference() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon_config = circom_poseidon_config();
        let params = CircomCRHParametersVar::new_constant(ns!(cs, "params"), &poseidon_config)?;

        let index_u64: u64 = 3;
        let index_value = Fr::from(index_u64);
        let prev_hash_chain_value =
            fr_from_hex("00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff");
        let value_value =
            fr_from_hex("00cafebabefeedfacecafebabefeedfacecafebabefeedfacecafebabefeedfa");
        let address_value = fr_from_hex("00112233445566778899aabbccddeeff00112233");
        let siblings_values = vec![11u64, 22, 33, 44]
            .into_iter()
            .map(Fr::from)
            .collect::<Vec<_>>();

        let prev_root_value =
            merkle_root_from_path(&poseidon_config, index_u64, Fr::zero(), &siblings_values);
        let expected_prev_root = fr_from_hex(ROOT_TRANSITION_OLD_ROOT_HEX);
        assert_eq!(prev_root_value, expected_prev_root);

        let leaf_hash_value = circom_poseidon_hash(&poseidon_config, &[address_value, value_value]);
        let expected_leaf_hash = fr_from_hex(ROOT_TRANSITION_LEAF_HASH_HEX);
        assert_eq!(leaf_hash_value, expected_leaf_hash);

        let expected_new_root = merkle_root_from_path(
            &poseidon_config,
            index_u64,
            leaf_hash_value,
            &siblings_values,
        );
        let expected_new_root_const = fr_from_hex(ROOT_TRANSITION_NEW_ROOT_HEX);
        assert_eq!(expected_new_root, expected_new_root_const);
        let expected_new_hash_chain = hash_chain_reference(
            &hex_bytes("00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"),
            &hex_bytes("00112233445566778899aabbccddeeff00112233"),
            &hex_bytes("00cafebabefeedfacecafebabefeedfacecafebabefeedfacecafebabefeedfa"),
        );
        let expected_new_hash_chain_const = fr_from_hex(ROOT_TRANSITION_NEW_HASH_CHAIN_HEX);
        assert_eq!(expected_new_hash_chain, expected_new_hash_chain_const);

        let index = FpVar::<Fr>::new_witness(ns!(cs, "index"), || Ok(index_value))?;
        let prev_hash_chain =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_hash_chain"), || Ok(prev_hash_chain_value))?;
        let prev_root = FpVar::<Fr>::new_witness(ns!(cs, "prev_root"), || Ok(prev_root_value))?;
        let address = FpVar::<Fr>::new_witness(ns!(cs, "address"), || Ok(address_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let is_dummy = Boolean::constant(false);
        let (new_index, new_hash_chain, new_root) = root_transition_step::<Fr, DEPTH>(
            &params,
            &index,
            &prev_hash_chain,
            &prev_root,
            &address,
            &value,
            &siblings,
            &is_dummy,
        )?;

        let expected_index_var =
            FpVar::<Fr>::new_input(ns!(cs, "new_index"), || Ok(Fr::from(index_u64 + 1)))?;
        let expected_hash_chain_var = FpVar::<Fr>::new_input(ns!(cs, "new_hash_chain"), || {
            Ok(expected_new_hash_chain_const)
        })?;
        let expected_root_var =
            FpVar::<Fr>::new_input(ns!(cs, "new_root"), || Ok(expected_new_root_const))?;

        new_index.enforce_equal(&expected_index_var)?;
        new_hash_chain.enforce_equal(&expected_hash_chain_var)?;
        new_root.enforce_equal(&expected_root_var)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn root_transition_dummy_passthrough() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon_config = circom_poseidon_config();
        let params = CircomCRHParametersVar::new_constant(ns!(cs, "params"), &poseidon_config)?;

        let index = FpVar::<Fr>::new_witness(ns!(cs, "index"), || Ok(Fr::from(5u64)))?;
        let prev_hash_chain = FpVar::<Fr>::new_witness(ns!(cs, "prev_hash_chain"), || {
            Ok(fr_from_hex(
                "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
            ))
        })?;
        let prev_root = FpVar::<Fr>::new_witness(ns!(cs, "prev_root"), || {
            Ok(fr_from_hex(
                "0x081afc0a0fce793ea0607b8d9961e73fefc2edb04bf8b61b5cd0456a34d89080",
            ))
        })?;
        let address = FpVar::<Fr>::new_witness(ns!(cs, "address"), || Ok(Fr::from(10u64)))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(Fr::from(20u64)))?;
        let siblings = (0..DEPTH)
            .map(|i| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(Fr::from(i as u64))))
            .collect::<Result<Vec<_>, _>>()?;

        let is_dummy = Boolean::constant(true);
        let (new_index, new_hash_chain, new_root) = root_transition_step::<Fr, DEPTH>(
            &params,
            &index,
            &prev_hash_chain,
            &prev_root,
            &address,
            &value,
            &siblings,
            &is_dummy,
        )?;

        new_index.enforce_equal(&index)?;
        new_hash_chain.enforce_equal(&prev_hash_chain)?;
        new_root.enforce_equal(&prev_root)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    fn fr_from_hex(hex: &str) -> Fr {
        Fr::from_be_bytes_mod_order(&hex_bytes(hex))
    }

    fn hex_bytes(hex: &str) -> Vec<u8> {
        let clean = hex.trim_start_matches("0x");
        let mut bytes = Vec::with_capacity((clean.len() + 1) / 2);
        let mut chars = clean.chars();
        while let Some(high) = chars.next() {
            let low = chars.next().unwrap_or('0');
            let byte = u8::from_str_radix(&format!("{}{}", high, low), 16).unwrap();
            bytes.push(byte);
        }
        bytes
    }

    fn hash_chain_reference(prev: &[u8], address: &[u8], value: &[u8]) -> Fr {
        let mut hasher = Sha256::new();
        hasher.update(prev);
        hasher.update(address);
        hasher.update(value);
        let digest = hasher.finalize();
        Fr::from_be_bytes_mod_order(&digest[1..])
    }
}
