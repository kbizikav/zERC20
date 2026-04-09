// SPDX-License-Identifier: BUSL-1.1

use crate::{
    circuits::constants::{ADDRESS_BIT_LENGTH, POW_DIFFICULTY},
    utils::poseidon::{
        gadgets::{CircomCRHParametersVar, poseidon3_var},
        utils::poseidon3,
    },
};
use ark_bn254::Fr;
use ark_crypto_primitives::sponge::Absorb;
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

const BURN_ADDRESS_DOMAIN: [u8; 4] = *b"burn";

#[derive(Debug, Error)]
pub enum BurnAddressError {
    #[error(
        "poseidon hash does not satisfy the required PoW difficulty of {difficulty} leading zero bits"
    )]
    PowDifficultyUnsatisfied { difficulty: usize },
}

pub(crate) fn burn_address_domain<F: PrimeField>() -> F {
    let mut domain_bytes = [0u8; 32];
    domain_bytes[..BURN_ADDRESS_DOMAIN.len()].copy_from_slice(&BURN_ADDRESS_DOMAIN);
    F::from_le_bytes_mod_order(&domain_bytes)
}

fn poseidon_burn_address_hash(recipient: Fr, secret: Fr) -> Fr {
    let domain = burn_address_domain::<Fr>();
    poseidon3(domain, recipient, secret)
}

pub fn compute_burn_address_from_secret(recipient: Fr, secret: Fr) -> Result<Fr, BurnAddressError> {
    let hash_bigint = poseidon_burn_address_hash(recipient, secret).into_bigint();
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
    let domain = FpVar::<F>::constant(burn_address_domain::<F>());
    let poseidon = poseidon3_var(poseidon_params, &domain, recipient, secret)?;
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
        burn_address_domain, burn_address_var, compute_burn_address_from_secret, find_pow_nonce,
        secret_from_nonce,
    };
    use crate::{
        test_utils::truncate_to_160_bits,
        utils::poseidon::{
            gadgets::CircomCRHParametersVar,
            utils::{circom_poseidon_hash, circom_poseidon3_config},
        },
    };
    use ark_bn254::Fr;
    use ark_ff::PrimeField;
    use ark_r1cs_std::{alloc::AllocVar, boolean::Boolean, eq::EqGadget, fields::fp::FpVar};
    use ark_relations::{
        gr1cs::{ConstraintSystem, SynthesisError},
        ns,
    };
    use hex::decode;

    // Precomputed inputs that satisfy the PoW window so tests can avoid the expensive search.
    const ADDRESS_HASH_EXPECTED_HEX: &str =
        "0x0000000000000000000000003034a16d0e8b774fc609a8adcbe89ed3c5bad8c3";
    const FIXED_RECIPIENT: u64 = 123_456_789;
    const FIXED_SECRET_SEED: u64 = 1_000;
    const FIXED_NONCE: u64 = 138_276;
    const POW_SATISFYING_SECRET: u64 = FIXED_SECRET_SEED + FIXED_NONCE;

    fn pow_fixture() -> (Fr, Fr, Fr) {
        let recipient_value = Fr::from(FIXED_RECIPIENT);
        let secret_seed = Fr::from(FIXED_SECRET_SEED);
        let secret_value = secret_from_nonce(secret_seed, FIXED_NONCE);
        debug_assert_eq!(secret_value, Fr::from(POW_SATISFYING_SECRET));
        let expected_address = compute_burn_address_from_secret(recipient_value, secret_value)
            .expect("precomputed PoW should satisfy difficulty");
        (recipient_value, secret_value, expected_address)
    }

    #[test]
    fn burn_address_matches_reference() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();
        let (recipient_value, secret_value, expected_address) = pow_fixture();
        assert_eq!(find_pow_nonce(recipient_value, secret_value), 0);

        let config = circom_poseidon3_config();
        let params = CircomCRHParametersVar::new_constant(ns!(cs, "params"), &config)?;

        let recipient = FpVar::<Fr>::new_witness(ns!(cs, "recipient"), || Ok(recipient_value))?;
        let secret = FpVar::<Fr>::new_witness(ns!(cs, "secret"), || Ok(secret_value))?;
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_address))?;

        let should_constrain = Boolean::constant(true);
        let actual = burn_address_var(&params, &recipient, &secret, &should_constrain)?;
        actual.enforce_equal(&expected_var)?;

        assert!(cs.is_satisfied().unwrap());
        let domain = burn_address_domain::<Fr>();
        let host_expected = truncate_to_160_bits(circom_poseidon_hash(
            &config,
            &[domain, recipient_value, secret_value],
        ));
        assert_eq!(host_expected, expected_address);
        Ok(())
    }

    #[test]
    fn burn_address_matches_fixed_vector() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let (recipient_value, secret_value, pow_expected_field) = pow_fixture();

        let config = circom_poseidon3_config();
        let params = CircomCRHParametersVar::new_constant(ns!(cs, "params"), &config)?;

        let expected_bytes =
            decode(ADDRESS_HASH_EXPECTED_HEX.trim_start_matches("0x")).expect("valid hex constant");
        let expected_field = Fr::from_be_bytes_mod_order(&expected_bytes);
        assert_eq!(pow_expected_field, expected_field);

        let domain = burn_address_domain::<Fr>();
        let host_expected = truncate_to_160_bits(circom_poseidon_hash(
            &config,
            &[domain, recipient_value, secret_value],
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
