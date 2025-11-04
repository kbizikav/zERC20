use super::{CircomCRH, CircomTwoToOneCRH, circom_poseidon_hash};
use ark_crypto_primitives::crh::{CRHSchemeGadget as CRHGadgetTrait, TwoToOneCRHSchemeGadget};
use ark_crypto_primitives::sponge::{
    Absorb,
    constraints::CryptographicSpongeVar,
    poseidon::{PoseidonConfig, constraints::PoseidonSpongeVar},
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

impl<F: PrimeField + Absorb> CRHGadgetTrait<CircomCRH<F>, F> for CircomCRHGadget<F> {
    type InputVar = [FpVar<F>];
    type OutputVar = FpVar<F>;
    type ParametersVar = CircomCRHParametersVar<F>;

    fn evaluate(
        parameters: &Self::ParametersVar,
        input: &Self::InputVar,
    ) -> Result<Self::OutputVar, SynthesisError> {
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
        let cs = left_input.cs().or(right_input.cs());

        if cs.is_none() {
            let left = left_input.value()?;
            let right = right_input.value()?;
            return Ok(FpVar::Constant(circom_poseidon_hash(
                &parameters.parameters,
                &[left, right],
            )));
        }

        let mut sponge = PoseidonSpongeVar::new(cs, &parameters.parameters);
        sponge.absorb(left_input)?;
        sponge.absorb(right_input)?;

        let _ = sponge.squeeze_field_elements(1)?;
        Ok(sponge.state[0].clone())
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
