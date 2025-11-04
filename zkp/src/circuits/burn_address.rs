use crate::{
    circuits::constants::{ADDRESS_BIT_LENGTH, POW_DIFFICULTY},
    utils::poseidon::{
        gadgets::{CircomCRHGadget, CircomCRHParametersVar},
        utils::poseidon2,
    },
};
use ark_bn254::Fr;
use ark_crypto_primitives::{crh::CRHSchemeGadget, sponge::Absorb};
use ark_ff::{BigInteger, PrimeField};
use ark_r1cs_std::{
    boolean::Boolean,
    convert::ToBitsGadget,
    eq::EqGadget,
    fields::{FieldVar, fp::FpVar},
};
use ark_relations::gr1cs::SynthesisError;
use ark_std::vec::Vec;
use num_bigint::BigUint;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BurnAddressError {
    #[error(
        "poseidon hash does not satisfy the required PoW difficulty of {difficulty} leading zero bits"
    )]
    PowDifficultyUnsatisfied { difficulty: usize },
}

pub fn compute_burn_address_from_secret(recipient: Fr, secret: Fr) -> Result<Fr, BurnAddressError> {
    let hash_bigint = poseidon2(recipient, secret).into_bigint();
    let hash_bits = hash_bigint.to_bits_le();

    if hash_bits
        .iter()
        .skip(ADDRESS_BIT_LENGTH)
        .take(POW_DIFFICULTY)
        .any(|bit| *bit)
    {
        return Err(BurnAddressError::PowDifficultyUnsatisfied {
            difficulty: POW_DIFFICULTY,
        });
    }

    let hash = BigUint::from_bytes_le(&hash_bigint.to_bytes_le());
    let mask = (BigUint::from(1u8) << ADDRESS_BIT_LENGTH) - 1u8;
    let address = hash & mask;
    Ok(address.into())
}

pub fn find_pow_nonce(recipient: Fr, secret_seed: Fr) -> u64 {
    for nonce in 0u64.. {
        let candidate = secret_seed + Fr::from(nonce);
        if compute_burn_address_from_secret(recipient, candidate).is_ok() {
            return nonce;
        }
    }
    unreachable!("u64 nonce space exhausted while searching for PoW solution");
}

pub fn secret_from_nonce(secret_seed: Fr, nonce: u64) -> Fr {
    secret_seed + Fr::from(nonce)
}

pub fn compute_burn_address_for_nonce(
    recipient: Fr,
    secret_seed: Fr,
    nonce: u64,
) -> Result<Fr, BurnAddressError> {
    let secret = secret_from_nonce(secret_seed, nonce);
    compute_burn_address_from_secret(recipient, secret)
}

pub fn burn_address_var<F: PrimeField + Absorb>(
    poseidon_params: &CircomCRHParametersVar<F>,
    recipient: &FpVar<F>,
    secret: &FpVar<F>,
    is_constrained: &Boolean<F>,
) -> Result<FpVar<F>, SynthesisError> {
    let poseidon =
        CircomCRHGadget::<F>::evaluate(poseidon_params, &[recipient.clone(), secret.clone()])?;
    let poseidon_bits = poseidon.to_bits_le()?;

    let is_constrained_fp: FpVar<F> = is_constrained.clone().into();
    let zero = FpVar::<F>::constant(F::zero());
    for bit in poseidon_bits
        .iter()
        .skip(ADDRESS_BIT_LENGTH)
        .take(POW_DIFFICULTY)
    {
        let bit_fp: FpVar<F> = bit.clone().into();
        (bit_fp * is_constrained_fp.clone()).enforce_equal(&zero)?;
    }

    let truncated_bits: Vec<_> = poseidon_bits.into_iter().take(ADDRESS_BIT_LENGTH).collect();
    Boolean::le_bits_to_fp(&truncated_bits)
}

#[cfg(test)]
mod tests {
    use super::{
        burn_address_var, compute_burn_address_from_secret, find_pow_nonce, secret_from_nonce,
    };
    use crate::test_utils::truncate_to_160_bits;
    use crate::utils::poseidon::gadgets::CircomCRHParametersVar;
    use crate::utils::poseidon::utils::{circom_poseidon_config, circom_poseidon_hash};
    use ark_bn254::Fr;
    use ark_ff::PrimeField;
    use ark_r1cs_std::{alloc::AllocVar, boolean::Boolean, eq::EqGadget, fields::fp::FpVar};
    use ark_relations::gr1cs::{ConstraintSystem, SynthesisError};
    use ark_relations::ns;
    use ark_std::{rand::RngCore, test_rng};
    use hex::decode;

    const ADDRESS_HASH_EXPECTED_HEX: &str = "0x14c7a9ca62574b09437a1eaf1cc92a5aa869d2f8";

    fn sample_address_field(rng: &mut impl RngCore) -> Fr {
        let mut bytes = [0u8; 20];
        rng.fill_bytes(&mut bytes);
        Fr::from_be_bytes_mod_order(&bytes)
    }

    fn sample_secret_seed(rng: &mut impl RngCore) -> Fr {
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        Fr::from_be_bytes_mod_order(&bytes)
    }

    #[test]
    fn burn_address_matches_reference() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();
        let mut rng = test_rng();

        let recipient_value = sample_address_field(&mut rng);
        let secret_seed = sample_secret_seed(&mut rng);
        let nonce = find_pow_nonce(recipient_value, secret_seed);
        let secret_value = secret_from_nonce(secret_seed, nonce);
        let expected_address = compute_burn_address_from_secret(recipient_value, secret_value)
            .expect("nonce should satisfy PoW");

        let config = circom_poseidon_config();
        let params = CircomCRHParametersVar::new_constant(ns!(cs, "params"), &config)?;

        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_address))?;

        let should_constrain = Boolean::constant(true);
        let actual = burn_address_var(&params, &recipient, &secret, &should_constrain)?;
        actual.enforce_equal(&expected_var)?;

        assert!(cs.is_satisfied().unwrap());
        let host_expected = truncate_to_160_bits(circom_poseidon_hash(
            &config,
            &[recipient_value, secret_value],
        ));
        assert_eq!(host_expected, expected_address);
        Ok(())
    }

    #[test]
    fn burn_address_matches_fixed_vector() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let recipient_value = Fr::from(123_456_789u64);
        let secret_seed = Fr::from(1_000u64);
        let nonce = find_pow_nonce(recipient_value, secret_seed);
        let secret_value = secret_from_nonce(secret_seed, nonce);
        let pow_expected_field = compute_burn_address_from_secret(recipient_value, secret_value)
            .expect("nonce should satisfy PoW");

        let config = circom_poseidon_config();
        let params = CircomCRHParametersVar::new_constant(ns!(cs, "params"), &config)?;

        let expected_bytes =
            decode(ADDRESS_HASH_EXPECTED_HEX.trim_start_matches("0x")).expect("valid hex constant");
        let expected_field = Fr::from_be_bytes_mod_order(&expected_bytes);
        assert_eq!(pow_expected_field, expected_field);

        let host_expected = truncate_to_160_bits(circom_poseidon_hash(
            &config,
            &[recipient_value, secret_value],
        ));
        assert_eq!(host_expected, expected_field);

        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_field))?;

        let should_constrain = Boolean::constant(true);
        let actual = burn_address_var(&params, &recipient, &secret, &should_constrain)?;
        actual.enforce_equal(&expected_var)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn compute_burn_address_enforces_pow() {
        let recipient = Fr::from(42u64);
        let secret = Fr::from(17u64);
        match compute_burn_address_from_secret(recipient, secret) {
            Ok(_) => panic!("secret should not satisfy PoW"),
            Err(super::BurnAddressError::PowDifficultyUnsatisfied { difficulty }) => {
                assert_eq!(difficulty, super::POW_DIFFICULTY);
            }
        }
    }
}
