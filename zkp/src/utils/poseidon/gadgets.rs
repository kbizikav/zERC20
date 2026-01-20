use super::{CircomCRH, CircomTwoToOneCRH, circom_poseidon_hash};
use ark_crypto_primitives::{
    crh::{CRHSchemeGadget as CRHGadgetTrait, TwoToOneCRHSchemeGadget},
    sponge::{
        Absorb,
        constraints::CryptographicSpongeVar,
        poseidon::{PoseidonConfig, constraints::PoseidonSpongeVar},
    },
};
use ark_ff::PrimeField;
use ark_r1cs_std::{
    GR1CSVar,
    alloc::{AllocVar, AllocationMode},
    fields::fp::FpVar,
};
use ark_relations::gr1cs::{ConstraintSystemRef, Namespace, SynthesisError};
use ark_std::{borrow::Borrow, marker::PhantomData, vec::Vec};

#[derive(Clone)]
pub struct CircomCRHParametersVar<F: PrimeField + Absorb> {
    pub parameters: PoseidonConfig<F>,
}

pub struct CircomCRHGadget<F: PrimeField + Absorb> {
    field_phantom: PhantomData<F>,
}

pub fn poseidon_var<F: PrimeField + Absorb>(
    parameters: &CircomCRHParametersVar<F>,
    input: &[FpVar<F>],
) -> Result<FpVar<F>, SynthesisError> {
    let cs: ConstraintSystemRef<F> = input.cs();

    if cs.is_none() {
        let mut constants = Vec::with_capacity(input.len());
        for var in input.iter() {
            constants.push(var.value()?);
        }
        return Ok(FpVar::Constant(circom_poseidon_hash(
            &parameters.parameters,
            &constants,
        )));
    }

    let mut sponge = PoseidonSpongeVar::new(cs, &parameters.parameters);
    for value in input.iter() {
        sponge.absorb(value)?;
    }

    let _ = sponge.squeeze_field_elements(1)?;
    Ok(sponge.state[0].clone())
}

pub fn poseidon2_var<F: PrimeField + Absorb>(
    parameters: &CircomCRHParametersVar<F>,
    left: &FpVar<F>,
    right: &FpVar<F>,
) -> Result<FpVar<F>, SynthesisError> {
    poseidon_var(parameters, &[left.clone(), right.clone()])
}

pub fn poseidon3_var<F: PrimeField + Absorb>(
    parameters: &CircomCRHParametersVar<F>,
    left: &FpVar<F>,
    middle: &FpVar<F>,
    right: &FpVar<F>,
) -> Result<FpVar<F>, SynthesisError> {
    poseidon_var(parameters, &[left.clone(), middle.clone(), right.clone()])
}

impl<F: PrimeField + Absorb> CRHGadgetTrait<CircomCRH<F>, F> for CircomCRHGadget<F> {
    type InputVar = [FpVar<F>];
    type OutputVar = FpVar<F>;
    type ParametersVar = CircomCRHParametersVar<F>;

    fn evaluate(
        parameters: &Self::ParametersVar,
        input: &Self::InputVar,
    ) -> Result<Self::OutputVar, SynthesisError> {
        poseidon_var(parameters, input)
    }
}

pub struct CircomTwoToOneCRHGadget<F: PrimeField + Absorb> {
    field_phantom: PhantomData<F>,
}

impl<F: PrimeField + Absorb> TwoToOneCRHSchemeGadget<CircomTwoToOneCRH<F>, F>
    for CircomTwoToOneCRHGadget<F>
{
    type InputVar = FpVar<F>;
    type OutputVar = FpVar<F>;
    type ParametersVar = CircomCRHParametersVar<F>;

    fn evaluate(
        parameters: &Self::ParametersVar,
        left_input: &Self::InputVar,
        right_input: &Self::InputVar,
    ) -> Result<Self::OutputVar, SynthesisError> {
        Self::compress(parameters, left_input, right_input)
    }

    fn compress(
        parameters: &Self::ParametersVar,
        left_input: &Self::OutputVar,
        right_input: &Self::OutputVar,
    ) -> Result<Self::OutputVar, SynthesisError> {
        poseidon2_var(parameters, left_input, right_input)
    }
}

impl<F: PrimeField + Absorb> AllocVar<PoseidonConfig<F>, F> for CircomCRHParametersVar<F> {
    fn new_variable<T: Borrow<PoseidonConfig<F>>>(
        _cs: impl Into<Namespace<F>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
        _mode: AllocationMode,
    ) -> Result<Self, SynthesisError> {
        f().map(|param| {
            let parameters = param.borrow().clone();
            Self { parameters }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{CircomCRHParametersVar, poseidon2_var, poseidon3_var};
    use crate::utils::poseidon::utils::{
        circom_poseidon2_config, circom_poseidon3_config, poseidon2, poseidon3,
    };

    use ark_bn254::Fr;
    use ark_r1cs_std::{alloc::AllocVar, eq::EqGadget, fields::fp::FpVar};
    use ark_relations::{
        gr1cs::{ConstraintSystem, SynthesisError},
        ns,
    };

    #[test]
    fn poseidon2_var_matches_host() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let left_val = Fr::from(7u64);
        let right_val = Fr::from(8u64);

        let config = circom_poseidon2_config();
        let params = CircomCRHParametersVar::new_constant(ns!(cs, "params"), &config)?;

        let left = FpVar::<Fr>::new_witness(ns!(cs, "left"), || Ok(left_val))?;
        let right = FpVar::<Fr>::new_witness(ns!(cs, "right"), || Ok(right_val))?;
        let expected = poseidon2(left_val, right_val);
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected))?;

        let actual = poseidon2_var(&params, &left, &right)?;
        actual.enforce_equal(&expected_var)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }

    #[test]
    fn poseidon3_var_matches_host() -> Result<(), SynthesisError> {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let left_val = Fr::from(3u64);
        let middle_val = Fr::from(4u64);
        let right_val = Fr::from(5u64);

        let config = circom_poseidon3_config();
        let params = CircomCRHParametersVar::new_constant(ns!(cs, "params"), &config)?;

        let left = FpVar::<Fr>::new_witness(ns!(cs, "left"), || Ok(left_val))?;
        let middle = FpVar::<Fr>::new_witness(ns!(cs, "middle"), || Ok(middle_val))?;
        let right = FpVar::<Fr>::new_witness(ns!(cs, "right"), || Ok(right_val))?;
        let expected = poseidon3(left_val, middle_val, right_val);
        let expected_var = FpVar::<Fr>::new_input(ns!(cs, "expected"), || Ok(expected))?;

        let actual = poseidon3_var(&params, &left, &middle, &right)?;
        actual.enforce_equal(&expected_var)?;

        assert!(cs.is_satisfied().unwrap());
        Ok(())
    }
}
