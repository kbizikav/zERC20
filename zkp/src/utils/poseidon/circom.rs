use ark_crypto_primitives::{
    Error,
    crh::{CRHScheme, TwoToOneCRHScheme},
    sponge::{Absorb, poseidon::PoseidonConfig},
};
use ark_ff::PrimeField;
use ark_std::{borrow::Borrow, marker::PhantomData};
use rand::Rng;

use crate::utils::poseidon::circom_poseidon_hash;

/// Circom-compatible Poseidon CRH that outputs the first sponge limb.
pub struct CircomCRH<F: PrimeField + Absorb> {
    field_phantom: PhantomData<F>,
}

impl<F: PrimeField + Absorb> CRHScheme for CircomCRH<F> {
    type Input = [F];
    type Output = F;
    type Parameters = PoseidonConfig<F>;

    fn setup<R: Rng>(_r: &mut R) -> Result<Self::Parameters, Error> {
        unimplemented!("parameter generation is handled externally")
    }

    fn evaluate<T: Borrow<Self::Input>>(
        parameters: &Self::Parameters,
        input: T,
    ) -> Result<Self::Output, Error> {
        let input = input.borrow();
        if input.len() != parameters.rate {
            return Err(Error::IncorrectInputLength(input.len()));
        }

        Ok(circom_poseidon_hash(parameters, input))
    }
}

/// Circom-compatible Poseidon two-to-one hash that matches light-poseidon outputs.
pub struct CircomTwoToOneCRH<F: PrimeField + Absorb> {
    field_phantom: PhantomData<F>,
}

impl<F: PrimeField + Absorb> TwoToOneCRHScheme for CircomTwoToOneCRH<F> {
    type Input = F;
    type Output = F;
    type Parameters = PoseidonConfig<F>;

    fn setup<R: Rng>(_r: &mut R) -> Result<Self::Parameters, Error> {
        unimplemented!("parameter generation is handled externally")
    }

    fn evaluate<T: Borrow<Self::Input>>(
        parameters: &Self::Parameters,
        left_input: T,
        right_input: T,
    ) -> Result<Self::Output, Error> {
        Self::compress(parameters, left_input, right_input)
    }

    fn compress<T: Borrow<Self::Output>>(
        parameters: &Self::Parameters,
        left_input: T,
        right_input: T,
    ) -> Result<Self::Output, Error> {
        if parameters.rate != 2 {
            return Err(Error::IncorrectInputLength(parameters.rate));
        }

        let inputs = [left_input.borrow().clone(), right_input.borrow().clone()];
        Ok(circom_poseidon_hash(parameters, &inputs))
    }
}
