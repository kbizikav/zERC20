use crate::{
    circuits::{burn_address::burn_address_var, constants::BYTES31_BIT_LENGTH},
    utils::{
        poseidon::gadgets::CircomCRHParametersVar,
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

pub fn single_withdraw<F, const DEPTH: usize>(
    poseidon2_params: &CircomCRHParametersVar<F>,
    poseidon3_params: &CircomCRHParametersVar<F>,
    merkle_root: &FpVar<F>,
    recipient: &FpVar<F>,
    from_address: &FpVar<F>,
    value: &FpVar<F>,
    delta: &FpVar<F>,
    secret: &FpVar<F>,
    leaf_index: &FpVar<F>,
    siblings: &[FpVar<F>],
) -> Result<FpVar<F>, SynthesisError>
where
    F: PrimeField + Absorb,
{
    assert_eq!(siblings.len(), DEPTH);

    enforce_bit_length(leaf_index, DEPTH)?;
    enforce_bit_length(value, BYTES31_BIT_LENGTH)?;
    enforce_bit_length(delta, BYTES31_BIT_LENGTH)?;

    let index_bits = to_bits_le_limited(leaf_index, DEPTH)?;

    let enforce_pow = Boolean::constant(true);
    let leaf_address = burn_address_var(poseidon3_params, recipient, secret, &enforce_pow)?;
    let leaf_hash = leaf_hash_var(poseidon3_params, from_address, &leaf_address, value)?;
    let computed_root = merkle_root_from_leaf(poseidon2_params, &leaf_hash, &index_bits, siblings)?;

    merkle_root.enforce_equal(&computed_root)?;

    let withdraw_value = value.clone() - delta.clone();
    enforce_bit_length(&withdraw_value, BYTES31_BIT_LENGTH)?;

    Ok(withdraw_value)
}

pub fn withdraw_step<F, const DEPTH: usize>(
    poseidon2_params: &CircomCRHParametersVar<F>,
    poseidon3_params: &CircomCRHParametersVar<F>,
    merkle_root: &FpVar<F>,
    recipient: &FpVar<F>,
    from_address: &FpVar<F>,
    prev_leaf_index_with_offset: &FpVar<F>,
    prev_total_value: &FpVar<F>,
    is_dummy: &Boolean<F>,
    value: &FpVar<F>,
    secret: &FpVar<F>,
    leaf_index: &FpVar<F>,
    siblings: &[FpVar<F>],
) -> Result<(FpVar<F>, FpVar<F>, FpVar<F>, FpVar<F>), SynthesisError>
where
    F: PrimeField + Absorb,
{
    assert_eq!(siblings.len(), DEPTH);

    // input range checks
    let one = FpVar::<F>::constant(F::one());
    enforce_bit_length(leaf_index, DEPTH)?;
    enforce_bit_length(prev_leaf_index_with_offset, DEPTH + 1)?;
    enforce_bit_length(value, BYTES31_BIT_LENGTH)?;

    let leaf_index_with_offset = leaf_index.clone() + one.clone();

    enforce_strict_less_than(
        prev_leaf_index_with_offset,
        &leaf_index_with_offset,
        DEPTH + 1,
    )?;

    let index_bits = to_bits_le_limited(leaf_index, DEPTH)?;

    let should_constrain = is_dummy.clone().not();
    let leaf_address = burn_address_var(poseidon3_params, recipient, secret, &should_constrain)?;
    let leaf_hash = leaf_hash_var(poseidon3_params, from_address, &leaf_address, value)?;
    let computed_root = merkle_root_from_leaf(poseidon2_params, &leaf_hash, &index_bits, siblings)?;

    let is_dummy_fp: FpVar<F> = is_dummy.clone().into();
    let is_real_fp = one.clone() - is_dummy_fp.clone();

    let diff = merkle_root.clone() - computed_root;
    (diff * is_real_fp).enforce_equal(&FpVar::<F>::constant(F::zero()))?;

    let two = F::from(2u64);
    let factor = one - is_dummy_fp.clone() * FpVar::<F>::constant(two);
    let new_total_value = prev_total_value.clone() + value.clone() * factor;
    enforce_bit_length(&new_total_value, BYTES31_BIT_LENGTH)?;

    Ok((
        merkle_root.clone(),
        recipient.clone(),
        leaf_index_with_offset,
        new_total_value,
    ))
}

#[cfg(test)]
mod tests {
    use super::{single_withdraw, withdraw_step};
    use crate::{
        circuits::burn_address::{
            burn_address_domain, compute_burn_address_from_secret, secret_from_nonce,
        },
        test_utils::{merkle_root_from_path, truncate_to_160_bits},
        utils::poseidon::{
            gadgets::CircomCRHParametersVar,
            utils::{circom_poseidon_hash, circom_poseidon2_config, circom_poseidon3_config},
        },
    };
    use ark_bn254::Fr;
    use ark_ff::{PrimeField, Zero};
    use ark_r1cs_std::{alloc::AllocVar, boolean::Boolean, eq::EqGadget, fields::fp::FpVar};
    use ark_relations::{
        gr1cs::{ConstraintSystem, SynthesisError},
        ns,
    };
    use ark_std::vec::Vec;
    use hex::decode;

    const DEPTH: usize = 4;
    const WITHDRAW_LEAF_ADDRESS_HEX: &str =
        "0x00000000000000000000000026540d01ef93f4587b3d8a528ec57b30f9850c8f";
    const WITHDRAW_LEAF_HASH_HEX: &str =
        "0x043c26c079e8bdfda7e4bdfc7918bb50972586792cce3f2e05a1109456859a31";
    const WITHDRAW_MERKLE_ROOT_HEX: &str =
        "0x297f924f707ecd7017231918ad292e2dd5b29a614cb4cc9bf1f2ab08d42cf7a6";

    struct PowFixture {
        recipient: u64,
        secret_seed: u64,
        nonce: u64,
        from: u64,
        address_hex: &'static str,
    }

    const REAL_LEAF_POW: PowFixture = PowFixture {
        recipient: 123,
        secret_seed: 456,
        nonce: 72_950,
        from: 789,
        address_hex: WITHDRAW_LEAF_ADDRESS_HEX,
    };
    const SINGLE_WITHDRAW_POW: PowFixture = PowFixture {
        recipient: 321,
        secret_seed: 654,
        nonce: 58_635,
        from: 555,
        address_hex: "0x000000000000000000000000edf728cc772812667eae4cdf4a764347857eb824",
    };
    const MAX_INDEX_POW: PowFixture = PowFixture {
        recipient: 42,
        secret_seed: 84,
        nonce: 91_789,
        from: 4242,
        address_hex: "0x00000000000000000000000089ff51a1dfa83573051fea53b9eb9cfa47e3bbb8",
    };
    const NON_INCREASING_POW: PowFixture = PowFixture {
        recipient: 1,
        secret_seed: 2,
        nonce: 4_105,
        from: 111,
        address_hex: "0x00000000000000000000000067589a9c2f969a6adfe4a9c2f51d797753a4d3f0",
    };

    fn load_pow_fixture(fixture: &PowFixture) -> (Fr, Fr, Fr, Fr) {
        let recipient_value = Fr::from(fixture.recipient);
        let secret_seed = Fr::from(fixture.secret_seed);
        let secret_value = secret_from_nonce(secret_seed, fixture.nonce);
        let expected_address = Fr::from_be_bytes_mod_order(
            &decode(fixture.address_hex.trim_start_matches("0x")).expect("valid burn address hex"),
        );
        let address_value =
            compute_burn_address_from_secret(recipient_value, secret_value).expect("valid PoW");
        assert_eq!(address_value, expected_address);
        (
            recipient_value,
            secret_value,
            address_value,
            Fr::from(fixture.from),
        )
    }

    #[test]
    fn single_withdraw_accepts_valid_leaf() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon3_config = circom_poseidon3_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;
        let poseidon3_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon3_params"), &poseidon3_config)?;

        let (recipient_value, secret_value, address_value, from_value) =
            load_pow_fixture(&SINGLE_WITHDRAW_POW);
        let value_value = Fr::from(100u64);
        let delta_value = Fr::from(25u64);
        let withdraw_value_value = value_value - delta_value;
        let leaf_index_u64 = 3u64;
        let leaf_index_value = Fr::from(leaf_index_u64);
        let siblings_values = vec![5u64, 6, 7, 8]
            .into_iter()
            .map(Fr::from)
            .collect::<Vec<_>>();

        let domain = burn_address_domain::<Fr>();
        let host_address = truncate_to_160_bits(circom_poseidon_hash(
            &poseidon3_config,
            &[domain, recipient_value, secret_value],
        ));
        assert_eq!(address_value, host_address);
        let leaf_value =
            circom_poseidon_hash(&poseidon3_config, &[from_value, address_value, value_value]);
        let merkle_root_value = merkle_root_from_path(
            &poseidon2_config,
            leaf_index_u64,
            leaf_value,
            &siblings_values,
        );

        let merkle_root =
            FpVar::<Fr>::new_witness(ns!(cs, "merkle_root"), || Ok(merkle_root_value))?;
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let from = FpVar::<Fr>::new_witness(ns!(cs, "from"), || Ok(from_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let delta = FpVar::<Fr>::new_witness(ns!(cs, "delta"), || Ok(delta_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let leaf_index = FpVar::<Fr>::new_witness(ns!(cs, "leaf_index"), || Ok(leaf_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let new_value = single_withdraw::<Fr, DEPTH>(
            &poseidon2_params,
            &poseidon3_params,
            &merkle_root,
            &recipient,
            &from,
            &value,
            &delta,
            &secret,
            &leaf_index,
            &siblings,
        )?;

        let expected_new_value =
            FpVar::<Fr>::new_input(ns!(cs, "expected_new_value"), || Ok(withdraw_value_value))?;
        new_value.enforce_equal(&expected_new_value)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn withdraw_real_leaf_updates_total() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon3_config = circom_poseidon3_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;
        let poseidon3_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon3_params"), &poseidon3_config)?;

        let (recipient_value, secret_value, address_value, from_value) =
            load_pow_fixture(&REAL_LEAF_POW);
        let value_value = Fr::from(7u64);
        let prev_leaf_index_with_offset_value = Fr::from(3u64);
        let leaf_index_u64 = 5u64;
        let leaf_index_value = Fr::from(leaf_index_u64);
        let leaf_index_with_offset_value = Fr::from(6u64);
        let prev_total_value_value = Fr::from(100u64);
        let is_dummy_value = false;

        let expected_address = Fr::from_be_bytes_mod_order(
            &decode(WITHDRAW_LEAF_ADDRESS_HEX.trim_start_matches("0x"))
                .expect("valid withdraw leaf address hex"),
        );
        assert_eq!(address_value, expected_address);
        let domain = burn_address_domain::<Fr>();
        let host_address = truncate_to_160_bits(circom_poseidon_hash(
            &poseidon3_config,
            &[domain, recipient_value, secret_value],
        ));
        assert_eq!(address_value, host_address);

        let leaf_value =
            circom_poseidon_hash(&poseidon3_config, &[from_value, address_value, value_value]);
        let expected_leaf = Fr::from_be_bytes_mod_order(
            &decode(WITHDRAW_LEAF_HASH_HEX.trim_start_matches("0x"))
                .expect("valid withdraw leaf hash hex"),
        );
        assert_eq!(leaf_value, expected_leaf);
        let siblings_values = vec![11u64, 22, 33, 44]
            .into_iter()
            .map(Fr::from)
            .collect::<Vec<_>>();
        let merkle_root_value = merkle_root_from_path(
            &poseidon2_config,
            leaf_index_u64,
            leaf_value,
            &siblings_values,
        );
        let expected_merkle_root = Fr::from_be_bytes_mod_order(
            &decode(WITHDRAW_MERKLE_ROOT_HEX.trim_start_matches("0x"))
                .expect("valid withdraw merkle root hex"),
        );
        assert_eq!(merkle_root_value, expected_merkle_root);
        let new_total_value_value = Fr::from(107u64);

        let merkle_root =
            FpVar::<Fr>::new_witness(ns!(cs, "merkle_root"), || Ok(merkle_root_value))?;
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let from = FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_value))?;
        let prev_leaf_index_with_offset =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_leaf_index_with_offset"), || {
                Ok(prev_leaf_index_with_offset_value)
            })?;
        let prev_total_value =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_total_value"), || Ok(prev_total_value_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(is_dummy_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let leaf_index = FpVar::<Fr>::new_witness(ns!(cs, "leaf_index"), || Ok(leaf_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let (out_root, out_recipient, out_leaf_index_with_offset, out_total) =
            withdraw_step::<Fr, DEPTH>(
                &poseidon2_params,
                &poseidon3_params,
                &merkle_root,
                &recipient,
                &from,
                &prev_leaf_index_with_offset,
                &prev_total_value,
                &is_dummy,
                &value,
                &secret,
                &leaf_index,
                &siblings,
            )?;

        let out_root_exp = FpVar::<Fr>::new_input(ns!(cs, "out_root"), || Ok(merkle_root_value))?;
        let out_recipient_exp =
            FpVar::<Fr>::new_input(ns!(cs, "out_recipient"), || Ok(recipient_value))?;
        let out_leaf_index_with_offset_exp =
            FpVar::<Fr>::new_input(ns!(cs, "out_leaf_index_with_offset"), || {
                Ok(leaf_index_with_offset_value)
            })?;
        let out_total_exp =
            FpVar::<Fr>::new_input(ns!(cs, "out_total"), || Ok(new_total_value_value))?;

        out_root.enforce_equal(&out_root_exp)?;
        out_recipient.enforce_equal(&out_recipient_exp)?;
        out_leaf_index_with_offset.enforce_equal(&out_leaf_index_with_offset_exp)?;
        out_total.enforce_equal(&out_total_exp)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn withdraw_accepts_max_leaf_index() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon3_config = circom_poseidon3_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;
        let poseidon3_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon3_params"), &poseidon3_config)?;

        let (recipient_value, secret_value, address_value, from_value) =
            load_pow_fixture(&MAX_INDEX_POW);
        let value_value = Fr::from(9u64);
        let prev_total_value_value = Fr::from(30u64);
        let leaf_index_u64 = (1u64 << DEPTH) - 1;
        let prev_leaf_index_with_offset_value = Fr::from(leaf_index_u64);
        let leaf_index_value = Fr::from(leaf_index_u64);
        let leaf_index_with_offset_value = Fr::from(leaf_index_u64 + 1);
        let is_dummy_value = false;

        let domain = burn_address_domain::<Fr>();
        let host_address = truncate_to_160_bits(circom_poseidon_hash(
            &poseidon3_config,
            &[domain, recipient_value, secret_value],
        ));
        assert_eq!(address_value, host_address);

        let siblings_values = vec![7u64, 9, 11, 13]
            .into_iter()
            .map(Fr::from)
            .collect::<Vec<_>>();
        let leaf_value =
            circom_poseidon_hash(&poseidon3_config, &[from_value, address_value, value_value]);
        let merkle_root_value = merkle_root_from_path(
            &poseidon2_config,
            leaf_index_u64,
            leaf_value,
            &siblings_values,
        );
        let new_total_value_value = prev_total_value_value + value_value;

        let merkle_root =
            FpVar::<Fr>::new_witness(ns!(cs, "merkle_root"), || Ok(merkle_root_value))?;
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let from = FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_value))?;
        let prev_leaf_index_with_offset =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_leaf_index_with_offset"), || {
                Ok(prev_leaf_index_with_offset_value)
            })?;
        let prev_total_value =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_total_value"), || Ok(prev_total_value_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(is_dummy_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let leaf_index = FpVar::<Fr>::new_witness(ns!(cs, "leaf_index"), || Ok(leaf_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let (out_root, out_recipient, out_leaf_index_with_offset, out_total) =
            withdraw_step::<Fr, DEPTH>(
                &poseidon2_params,
                &poseidon3_params,
                &merkle_root,
                &recipient,
                &from,
                &prev_leaf_index_with_offset,
                &prev_total_value,
                &is_dummy,
                &value,
                &secret,
                &leaf_index,
                &siblings,
            )?;

        let out_root_exp = FpVar::<Fr>::new_input(ns!(cs, "out_root"), || Ok(merkle_root_value))?;
        let out_recipient_exp =
            FpVar::<Fr>::new_input(ns!(cs, "out_recipient"), || Ok(recipient_value))?;
        let out_leaf_index_with_offset_exp =
            FpVar::<Fr>::new_input(ns!(cs, "out_leaf_index_with_offset"), || {
                Ok(leaf_index_with_offset_value)
            })?;
        let out_total_exp =
            FpVar::<Fr>::new_input(ns!(cs, "out_total"), || Ok(new_total_value_value))?;

        out_root.enforce_equal(&out_root_exp)?;
        out_recipient.enforce_equal(&out_recipient_exp)?;
        out_leaf_index_with_offset.enforce_equal(&out_leaf_index_with_offset_exp)?;
        out_total.enforce_equal(&out_total_exp)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn withdraw_dummy_skips_root_check() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon3_config = circom_poseidon3_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;
        let poseidon3_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon3_params"), &poseidon3_config)?;

        let recipient_value = Fr::from(10u64);
        let secret_value = Fr::from(20u64);
        let value_value = Fr::from(3u64);
        let from_value = Fr::from(30u64);
        let prev_leaf_index_with_offset_value = Fr::from(2u64);
        let leaf_index_value = Fr::from(2u64);
        let leaf_index_with_offset_value = Fr::from(3u64);
        let prev_total_value_value = Fr::from(50u64);
        let is_dummy_value = true;

        let siblings_values = vec![5u64, 6, 7, 8]
            .into_iter()
            .map(Fr::from)
            .collect::<Vec<_>>();
        let merkle_root_value = Fr::from(999_999_999u64);
        let new_total_value_value = Fr::from(47u64);

        let merkle_root =
            FpVar::<Fr>::new_witness(ns!(cs, "merkle_root"), || Ok(merkle_root_value))?;
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let from = FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_value))?;
        let prev_leaf_index_with_offset =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_leaf_index_with_offset"), || {
                Ok(prev_leaf_index_with_offset_value)
            })?;
        let prev_total_value =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_total_value"), || Ok(prev_total_value_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(is_dummy_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let leaf_index = FpVar::<Fr>::new_witness(ns!(cs, "leaf_index"), || Ok(leaf_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let (out_root, out_recipient, out_leaf_index_with_offset, out_total) =
            withdraw_step::<Fr, DEPTH>(
                &poseidon2_params,
                &poseidon3_params,
                &merkle_root,
                &recipient,
                &from,
                &prev_leaf_index_with_offset,
                &prev_total_value,
                &is_dummy,
                &value,
                &secret,
                &leaf_index,
                &siblings,
            )?;

        let out_root_exp = FpVar::<Fr>::new_input(ns!(cs, "out_root"), || Ok(merkle_root_value))?;
        let out_recipient_exp =
            FpVar::<Fr>::new_input(ns!(cs, "out_recipient"), || Ok(recipient_value))?;
        let out_leaf_index_with_offset_exp =
            FpVar::<Fr>::new_input(ns!(cs, "out_leaf_index_with_offset"), || {
                Ok(leaf_index_with_offset_value)
            })?;
        let out_total_exp =
            FpVar::<Fr>::new_input(ns!(cs, "out_total"), || Ok(new_total_value_value))?;

        out_root.enforce_equal(&out_root_exp)?;
        out_recipient.enforce_equal(&out_recipient_exp)?;
        out_leaf_index_with_offset.enforce_equal(&out_leaf_index_with_offset_exp)?;
        out_total.enforce_equal(&out_total_exp)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn withdraw_rejects_non_increasing_index() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let poseidon2_config = circom_poseidon2_config();
        let poseidon3_config = circom_poseidon3_config();
        let poseidon2_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon2_params"), &poseidon2_config)?;
        let poseidon3_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon3_params"), &poseidon3_config)?;

        let (recipient_value, secret_value, _, from_value) = load_pow_fixture(&NON_INCREASING_POW);
        let value_value = Fr::from(1u64);
        let prev_leaf_index_with_offset_value = Fr::from(6u64);
        let leaf_index_value = Fr::from(5u64);
        let leaf_index_with_offset_value = Fr::from(6u64);
        let prev_total_value_value = Fr::zero();
        let is_dummy_value = false;

        let siblings_values = vec![Fr::zero(); DEPTH];
        let merkle_root_value = Fr::zero();

        let merkle_root =
            FpVar::<Fr>::new_witness(ns!(cs, "merkle_root"), || Ok(merkle_root_value))?;
        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let from = FpVar::<Fr>::new_witness(ns!(cs, "from_address"), || Ok(from_value))?;
        let prev_leaf_index_with_offset =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_leaf_index_with_offset"), || {
                Ok(prev_leaf_index_with_offset_value)
            })?;
        let prev_total_value =
            FpVar::<Fr>::new_witness(ns!(cs, "prev_total_value"), || Ok(prev_total_value_value))?;
        let is_dummy = Boolean::new_witness(ns!(cs, "is_dummy"), || Ok(is_dummy_value))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let leaf_index = FpVar::<Fr>::new_witness(ns!(cs, "leaf_index"), || Ok(leaf_index_value))?;
        let siblings = siblings_values
            .iter()
            .map(|s| FpVar::<Fr>::new_witness(ns!(cs, "sibling"), || Ok(*s)))
            .collect::<Result<Vec<_>, _>>()?;

        let (out_root, out_recipient, out_leaf_index_with_offset, out_total) =
            withdraw_step::<Fr, DEPTH>(
                &poseidon2_params,
                &poseidon3_params,
                &merkle_root,
                &recipient,
                &from,
                &prev_leaf_index_with_offset,
                &prev_total_value,
                &is_dummy,
                &value,
                &secret,
                &leaf_index,
                &siblings,
            )?;

        let out_root_exp = FpVar::<Fr>::new_input(ns!(cs, "out_root"), || Ok(merkle_root_value))?;
        let out_recipient_exp =
            FpVar::<Fr>::new_input(ns!(cs, "out_recipient"), || Ok(recipient_value))?;
        let out_leaf_index_with_offset_exp =
            FpVar::<Fr>::new_input(ns!(cs, "out_leaf_index_with_offset"), || {
                Ok(leaf_index_with_offset_value)
            })?;
        let out_total_exp =
            FpVar::<Fr>::new_input(ns!(cs, "out_total"), || Ok(prev_total_value_value))?;

        out_root.enforce_equal(&out_root_exp)?;
        out_recipient.enforce_equal(&out_recipient_exp)?;
        out_leaf_index_with_offset.enforce_equal(&out_leaf_index_with_offset_exp)?;
        out_total.enforce_equal(&out_total_exp)?;

        assert!(!cs.is_satisfied().unwrap());
        Ok(())
    }
}
