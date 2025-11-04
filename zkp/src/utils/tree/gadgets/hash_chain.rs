use alloy::primitives::{Address, U256};
use ark_crypto_primitives::crh::sha256::constraints::Sha256Gadget;
use ark_ff::PrimeField;
use ark_r1cs_std::{boolean::Boolean, fields::fp::FpVar, prelude::*, uint8::UInt8};
use ark_relations::gr1cs::SynthesisError;
use ark_std::vec::Vec;
use sha2::{Digest as _, Sha256};

pub fn hash_chain(prev: U256, addr: Address, value: U256) -> U256 {
    let mut digest: [u8; 32] = Sha256::digest(
        [
            prev.to_be_bytes_vec(),
            addr.0.0.to_vec(),
            value.to_be_bytes_vec(),
        ]
        .concat(),
    )
    .into();
    // Zero the most-significant byte (big-endian) -> keep lower 248 bits
    digest[0] = 0;
    U256::from_be_slice(&digest)
}

pub fn hash_chain_var<F: PrimeField>(
    prev_hash_chain: &FpVar<F>,
    address: &FpVar<F>,
    value: &FpVar<F>,
) -> Result<FpVar<F>, SynthesisError> {
    let (prev_prefix, mut prev_bytes) = fp_var_be_bytes(prev_hash_chain, 32)?;
    enforce_zero_bytes(&prev_prefix)?;
    if let Some(first) = prev_bytes.first() {
        first.enforce_equal(&UInt8::constant(0))?;
    }

    let (address_prefix, address_bytes) = fp_var_be_bytes(address, 20)?;
    enforce_zero_bytes(&address_prefix)?;

    let (value_prefix, mut value_bytes) = fp_var_be_bytes(value, 32)?;
    enforce_zero_bytes(&value_prefix)?;
    if let Some(first) = value_bytes.first() {
        first.enforce_equal(&UInt8::constant(0))?;
    }

    let mut sha_input = Vec::with_capacity(32 + 20 + 32);
    sha_input.append(&mut prev_bytes);
    sha_input.extend_from_slice(&address_bytes);
    sha_input.append(&mut value_bytes);

    let digest = Sha256Gadget::<F>::digest(sha_input.as_slice())?;
    let digest_bytes = digest.0;
    debug_assert_eq!(digest_bytes.len(), 32);

    let truncated = &digest_bytes[1..];
    let mut new_hash_bits = Vec::with_capacity(truncated.len() * 8);
    for byte in truncated.iter().rev() {
        new_hash_bits.extend(byte.to_bits_le()?);
    }

    Boolean::le_bits_to_fp(&new_hash_bits)
}

fn fp_var_be_bytes<F: PrimeField>(
    var: &FpVar<F>,
    target_len: usize,
) -> Result<(Vec<UInt8<F>>, Vec<UInt8<F>>), SynthesisError> {
    let bytes_le = var.to_bytes_le()?;
    let mut bytes_be: Vec<_> = bytes_le.into_iter().rev().collect();
    if bytes_be.len() < target_len {
        let mut pad = vec![UInt8::constant(0); target_len - bytes_be.len()];
        pad.extend(bytes_be);
        bytes_be = pad;
    }
    let split = bytes_be.len().saturating_sub(target_len);
    let prefix = bytes_be[..split].to_vec();
    let body = bytes_be[split..].to_vec();
    Ok((prefix, body))
}

fn enforce_zero_bytes<F: PrimeField>(bytes: &[UInt8<F>]) -> Result<(), SynthesisError> {
    for byte in bytes {
        byte.enforce_equal(&UInt8::constant(0))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::hash_chain_var;
    use ark_bn254::Fr;
    use ark_ff::PrimeField;
    use ark_r1cs_std::{alloc::AllocVar, eq::EqGadget, fields::fp::FpVar};
    use ark_relations::gr1cs::{ConstraintSystem, SynthesisError};
    use ark_relations::ns;
    use ark_std::{rand::RngCore, test_rng};
    use hex::decode;
    use sha2::{Digest, Sha256};

    const ZERO_VECTOR_EXPECTED_HEX: &str =
        "0xea5e6a3ec5f5474a26d858bc77b6d7bd3ab864ea02d988683fdc648602b248";

    fn sample_248_bit_field(rng: &mut impl RngCore) -> (Fr, Vec<u8>) {
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        bytes[0] = 0;
        let field = Fr::from_be_bytes_mod_order(&bytes);
        (field, bytes.to_vec())
    }

    fn sample_address_field(rng: &mut impl RngCore) -> (Fr, Vec<u8>) {
        let mut bytes = [0u8; 20];
        rng.fill_bytes(&mut bytes);
        let field = Fr::from_be_bytes_mod_order(&bytes);
        (field, bytes.to_vec())
    }

    fn expected_hash(prev: &[u8], addr: &[u8], value: &[u8]) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(prev);
        hasher.update(addr);
        hasher.update(value);
        let digest = hasher.finalize();
        digest[1..].to_vec()
    }

    #[test]
    fn hash_chain_matches_reference() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();
        let mut rng = test_rng();

        let (prev_field, prev_bytes) = sample_248_bit_field(&mut rng);
        let (value_field, value_bytes) = sample_248_bit_field(&mut rng);
        let (addr_field, addr_bytes) = sample_address_field(&mut rng);

        let expected_bytes = expected_hash(&prev_bytes, &addr_bytes, &value_bytes);
        let expected_field = Fr::from_be_bytes_mod_order(&expected_bytes);

        let prev_hash_chain = FpVar::<Fr>::new_witness(ns!(cs, "prev"), || Ok(prev_field))?;
        let address = FpVar::<Fr>::new_witness(ns!(cs, "addr"), || Ok(addr_field))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_field))?;
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_field))?;

        let actual = hash_chain_var(&prev_hash_chain, &address, &value)?;
        actual.enforce_equal(&expected_var)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn hash_chain_matches_zero_constants() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let prev_bytes = vec![0u8; 32];
        let addr_bytes = vec![0u8; 20];
        let value_bytes = vec![0u8; 32];

        let expected_bytes = expected_hash(&prev_bytes, &addr_bytes, &value_bytes);
        let expected_constant = decode(ZERO_VECTOR_EXPECTED_HEX.trim_start_matches("0x"))
            .expect("valid zero-vector hex constant");
        assert_eq!(expected_bytes, expected_constant);

        let prev_hash_chain = FpVar::<Fr>::Constant(Fr::from(0u64));
        let address = FpVar::<Fr>::Constant(Fr::from(0u64));
        let value = FpVar::<Fr>::Constant(Fr::from(0u64));
        let expected_var = FpVar::<Fr>::Constant(Fr::from_be_bytes_mod_order(&expected_constant));

        let actual = hash_chain_var(&prev_hash_chain, &address, &value)?;
        actual.enforce_equal(&expected_var)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn hash_chain_detects_wrong_output() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();
        let mut rng = test_rng();

        let (prev_field, prev_bytes) = sample_248_bit_field(&mut rng);
        let (value_field, value_bytes) = sample_248_bit_field(&mut rng);
        let (addr_field, addr_bytes) = sample_address_field(&mut rng);

        let expected_bytes = expected_hash(&prev_bytes, &addr_bytes, &value_bytes);
        let mut wrong_bytes = expected_bytes.clone();
        wrong_bytes[30] ^= 0x01;
        let wrong_field = Fr::from_be_bytes_mod_order(&wrong_bytes);

        let prev_hash_chain = FpVar::<Fr>::new_witness(ns!(cs, "prev"), || Ok(prev_field))?;
        let address = FpVar::<Fr>::new_witness(ns!(cs, "addr"), || Ok(addr_field))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_field))?;
        let wrong_var = FpVar::<Fr>::new_input(ns!(cs, "wrong"), || Ok(wrong_field))?;

        let actual = hash_chain_var(&prev_hash_chain, &address, &value)?;
        actual.enforce_equal(&wrong_var)?;

        assert!(!cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn hash_chain_rejects_out_of_range_inputs() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();
        let mut rng = test_rng();

        let mut prev_bytes = [0u8; 32];
        rng.fill_bytes(&mut prev_bytes);
        prev_bytes[0] = 1;
        let prev_field = Fr::from_be_bytes_mod_order(&prev_bytes);

        let (value_field, value_bytes) = sample_248_bit_field(&mut rng);
        let (addr_field, addr_bytes) = sample_address_field(&mut rng);

        let expected_bytes = expected_hash(&prev_bytes, &addr_bytes, &value_bytes);
        let expected_field = Fr::from_be_bytes_mod_order(&expected_bytes);

        let prev_hash_chain = FpVar::<Fr>::new_witness(ns!(cs, "prev"), || Ok(prev_field))?;
        let address = FpVar::<Fr>::new_witness(ns!(cs, "addr"), || Ok(addr_field))?;
        let value = FpVar::<Fr>::new_witness(ns!(cs, "value"), || Ok(value_field))?;
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected_field))?;

        let actual = hash_chain_var(&prev_hash_chain, &address, &value)?;
        actual.enforce_equal(&expected_var)?;

        assert!(!cs.is_satisfied().unwrap());
        Ok(())
    }
}
