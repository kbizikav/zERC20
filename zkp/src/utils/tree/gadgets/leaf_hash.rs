use ark_bn254::Fr;
use ark_crypto_primitives::{crh::CRHSchemeGadget, sponge::Absorb};
use ark_ff::PrimeField;
use ark_r1cs_std::fields::fp::FpVar;
use ark_relations::gr1cs::SynthesisError;

use crate::utils::poseidon::{
    gadgets::{CircomCRHGadget, CircomCRHParametersVar},
    utils::poseidon2,
};

pub fn compute_leaf_hash(address: Fr, amount: Fr) -> Fr {
    poseidon2(address, amount)
}

pub fn leaf_hash_var<F: PrimeField + Absorb>(
    poseidon_params: &CircomCRHParametersVar<F>,
    addr: &FpVar<F>,
    amount: &FpVar<F>,
) -> Result<FpVar<F>, SynthesisError> {
    CircomCRHGadget::<F>::evaluate(poseidon_params, &[addr.clone(), amount.clone()])
}

#[cfg(test)]
mod tests {
    use crate::utils::poseidon::circom_poseidon_hash;
    use crate::utils::poseidon::gadgets::CircomCRHParametersVar;
    use crate::utils::poseidon::utils::circom_poseidon_config;

    use super::leaf_hash_var;
    use ark_bn254::Fr;
    use ark_ff::PrimeField;
    use ark_r1cs_std::{alloc::AllocVar, eq::EqGadget, fields::fp::FpVar};
    use ark_relations::gr1cs::{ConstraintSystem, SynthesisError};
    use ark_relations::ns;
    use ark_std::{rand::RngCore, test_rng};
    use hex::decode;

    const LEAF_HASH_SMALL_HEX: &str =
        "0x115cc0f5e7d690413df64c6b9662e9cf2a3617f2743245519e19607a4417189a";
    const LEAF_HASH_LARGE_HEX: &str =
        "0x29003649e4aafd1d71eef6edfdb6a783e209d5b323a0da5aa16cdc9437cc266a";

    fn sample_field(rng: &mut impl RngCore) -> Fr {
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        Fr::from_be_bytes_mod_order(&bytes)
    }

    #[test]
    fn leaf_hash_matches_reference() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();
        let mut rng = test_rng();

        let addr_val = sample_field(&mut rng);
        let amount_val = sample_field(&mut rng);

        let config = circom_poseidon_config();
        let params = CircomCRHParametersVar::new_constant(ns!(cs, "params"), &config)?;

        let addr = FpVar::<Fr>::new_witness(ns!(cs, "addr"), || Ok(addr_val))?;
        let amount = FpVar::<Fr>::new_witness(ns!(cs, "amount"), || Ok(amount_val))?;
        let expected = circom_poseidon_hash(&config, &[addr_val, amount_val]);
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected))?;

        let actual = leaf_hash_var(&params, &addr, &amount)?;
        actual.enforce_equal(&expected_var)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn leaf_hash_detects_wrong_output() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();
        let mut rng = test_rng();

        let addr_val = sample_field(&mut rng);
        let amount_val = sample_field(&mut rng);

        let config = circom_poseidon_config();
        let params = CircomCRHParametersVar::new_constant(ns!(cs, "params"), &config)?;

        let addr = FpVar::<Fr>::new_witness(ns!(cs, "addr"), || Ok(addr_val))?;
        let amount = FpVar::<Fr>::new_witness(ns!(cs, "amount"), || Ok(amount_val))?;
        let expected = circom_poseidon_hash(&config, &[addr_val, amount_val]) + Fr::from(1u64);
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected))?;

        let actual = leaf_hash_var(&params, &addr, &amount)?;
        actual.enforce_equal(&expected_var)?;

        assert!(!cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn leaf_hash_matches_small_fixed_vector() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let addr_val = Fr::from(1u64);
        let amount_val = Fr::from(2u64);

        let config = circom_poseidon_config();
        let params = CircomCRHParametersVar::new_constant(ns!(cs, "params"), &config)?;

        let expected_bytes =
            decode(LEAF_HASH_SMALL_HEX.trim_start_matches("0x")).expect("valid small vector hex");
        let expected_field = Fr::from_be_bytes_mod_order(&expected_bytes);

        let host_expected = circom_poseidon_hash(&config, &[addr_val, amount_val]);
        assert_eq!(host_expected, expected_field);

        let addr = FpVar::<Fr>::new_witness(ns!(cs, "addr"), || Ok(addr_val))?;
        let amount = FpVar::<Fr>::new_witness(ns!(cs, "amount"), || Ok(amount_val))?;
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_field))?;

        let actual = leaf_hash_var(&params, &addr, &amount)?;
        actual.enforce_equal(&expected_var)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn leaf_hash_matches_large_fixed_vector() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let addr_val = Fr::from(123_456_789u64);
        let amount_val = Fr::from(1_000u64);

        let config = circom_poseidon_config();
        let params = CircomCRHParametersVar::new_constant(ns!(cs, "params"), &config)?;

        let expected_bytes =
            decode(LEAF_HASH_LARGE_HEX.trim_start_matches("0x")).expect("valid large vector hex");
        let expected_field = Fr::from_be_bytes_mod_order(&expected_bytes);

        let host_expected = circom_poseidon_hash(&config, &[addr_val, amount_val]);
        assert_eq!(host_expected, expected_field);

        let addr = FpVar::<Fr>::new_witness(ns!(cs, "addr"), || Ok(addr_val))?;
        let amount = FpVar::<Fr>::new_witness(ns!(cs, "amount"), || Ok(amount_val))?;
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_field))?;

        let actual = leaf_hash_var(&params, &addr, &amount)?;
        actual.enforce_equal(&expected_var)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }
}
