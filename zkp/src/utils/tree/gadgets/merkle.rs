use crate::utils::poseidon::gadgets::{CircomCRHGadget, CircomCRHParametersVar};
use ark_crypto_primitives::{crh::CRHSchemeGadget as _, sponge::Absorb};
use ark_ff::PrimeField;
use ark_r1cs_std::{
    boolean::Boolean,
    convert::ToBitsGadget,
    eq::EqGadget,
    fields::{FieldVar, fp::FpVar},
};
use ark_relations::gr1cs::SynthesisError;

pub fn to_bits_le_limited<F: PrimeField>(
    value: &FpVar<F>,
    num_bits: usize,
) -> Result<Vec<Boolean<F>>, SynthesisError> {
    let bits = value.to_bits_le()?;
    for bit in bits.iter().skip(num_bits) {
        bit.enforce_equal(&Boolean::constant(false))?;
    }
    Ok(bits.into_iter().take(num_bits).collect())
}

pub fn enforce_bit_length<F: PrimeField>(
    value: &FpVar<F>,
    num_bits: usize,
) -> Result<(), SynthesisError> {
    let _ = to_bits_le_limited(value, num_bits)?;
    Ok(())
}

/// Enforce `left < right` by constraining both operands to `num_bits` and
/// observing that `left + 2^num_bits - right` overflows iff `left >= right`.
pub fn enforce_strict_less_than<F: PrimeField>(
    left: &FpVar<F>,
    right: &FpVar<F>,
    num_bits: usize,
) -> Result<(), SynthesisError> {
    enforce_bit_length(left, num_bits)?;
    enforce_bit_length(right, num_bits)?;

    let mut offset = F::one();
    for _ in 0..num_bits {
        offset += offset;
    }
    let offset_var = FpVar::constant(offset);

    let adjusted = left + offset_var - right;
    let bits = adjusted.to_bits_le()?;
    let bit = bits
        .get(num_bits)
        .ok_or(SynthesisError::Unsatisfiable)?
        .clone();
    bit.enforce_equal(&Boolean::constant(false))
}

pub fn merkle_root_from_leaf<F: PrimeField + Absorb>(
    poseidon_params: &CircomCRHParametersVar<F>,
    leaf: &FpVar<F>,
    index_bits: &[Boolean<F>],
    siblings: &[FpVar<F>],
) -> Result<FpVar<F>, SynthesisError> {
    assert_eq!(index_bits.len(), siblings.len());

    let mut current = leaf.clone();
    for (bit, sibling) in index_bits.iter().zip(siblings.iter()) {
        let bit_fp: FpVar<F> = bit.clone().into();
        let one_minus_bit = FpVar::constant(F::one()) - bit_fp.clone();

        let left = current.clone() * one_minus_bit.clone() + sibling.clone() * bit_fp.clone();
        let right = current.clone() * bit_fp.clone() + sibling.clone() * one_minus_bit;

        current = CircomCRHGadget::<F>::evaluate(poseidon_params, &[left, right])?;
    }

    Ok(current)
}
