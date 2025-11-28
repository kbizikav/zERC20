use alloy::primitives::U256;
use ark_bn254::Fr;
use ark_crypto_primitives::sponge::{Absorb, poseidon::PoseidonConfig};
use ark_ff::{AdditiveGroup as _, Field as _, PrimeField};
use ark_r1cs_std::{
    alloc::{AllocVar, AllocationMode},
    fields::fp::FpVar,
    prelude::Boolean,
};
use ark_relations::gr1cs::{ConstraintSystemRef, Namespace, SynthesisError};
use ark_std::vec::Vec;
use core::{borrow::Borrow, convert::TryInto};
use folding_schemes::{Error, frontend::FCircuit};

use crate::{
    circuits::withdraw::withdraw_step,
    utils::{convertion::u256_to_fr, poseidon::gadgets::CircomCRHParametersVar},
};

pub const WITHDRAW_STATE_LEN: usize = 4;

#[derive(Clone, Debug)]
pub struct WithdrawCircuit<F: PrimeField + Absorb, const DEPTH: usize> {
    pub poseidon_params: PoseidonConfig<F>,
}

#[derive(Clone, Debug)]
pub struct WithdrawExternalInputs<F: PrimeField, const DEPTH: usize> {
    pub is_dummy: F,
    pub value: F,
    pub secret: F,
    pub leaf_index: F,
    pub siblings: [F; DEPTH],
}

impl<F: PrimeField, const DEPTH: usize> Default for WithdrawExternalInputs<F, DEPTH> {
    fn default() -> Self {
        Self {
            is_dummy: F::zero(),
            value: F::zero(),
            secret: F::zero(),
            leaf_index: F::zero(),
            siblings: core::array::from_fn(|_| F::zero()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct WithdrawExternalInputsVar<F: PrimeField, const DEPTH: usize> {
    pub is_dummy: Boolean<F>,
    pub value: FpVar<F>,
    pub secret: FpVar<F>,
    pub leaf_index: FpVar<F>,
    pub siblings: [FpVar<F>; DEPTH],
}

impl<F: PrimeField, const DEPTH: usize> AllocVar<WithdrawExternalInputs<F, DEPTH>, F>
    for WithdrawExternalInputsVar<F, DEPTH>
{
    fn new_variable<T: Borrow<WithdrawExternalInputs<F, DEPTH>>>(
        cs: impl Into<Namespace<F>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
        mode: AllocationMode,
    ) -> Result<Self, SynthesisError> {
        let ns = cs.into();
        let cs = ns.cs();
        f().and_then(|value| {
            let value = value.borrow();
            let is_dummy = Boolean::new_variable(
                cs.clone(),
                || {
                    if value.is_dummy.is_zero() {
                        Ok(false)
                    } else if value.is_dummy.is_one() {
                        Ok(true)
                    } else {
                        Err(SynthesisError::AssignmentMissing)
                    }
                },
                mode,
            )?;
            let val = FpVar::<F>::new_variable(cs.clone(), || Ok(value.value), mode)?;
            let secret = FpVar::<F>::new_variable(cs.clone(), || Ok(value.secret), mode)?;
            let leaf_index = FpVar::<F>::new_variable(cs.clone(), || Ok(value.leaf_index), mode)?;
            let siblings = <[FpVar<F>; DEPTH] as AllocVar<[F; DEPTH], F>>::new_variable(
                cs,
                || Ok(value.siblings.clone()),
                mode,
            )?;
            Ok(Self {
                is_dummy,
                value: val,
                secret,
                leaf_index,
                siblings,
            })
        })
    }
}

impl<F: PrimeField + Absorb, const DEPTH: usize> FCircuit<F> for WithdrawCircuit<F, DEPTH> {
    type Params = PoseidonConfig<F>;
    type ExternalInputs = WithdrawExternalInputs<F, DEPTH>;
    type ExternalInputsVar = WithdrawExternalInputsVar<F, DEPTH>;

    fn new(params: Self::Params) -> Result<Self, Error> {
        Ok(Self {
            poseidon_params: params,
        })
    }

    fn state_len(&self) -> usize {
        WITHDRAW_STATE_LEN
    }

    fn generate_step_constraints(
        &self,
        _cs: ConstraintSystemRef<F>,
        _i: usize,
        z_i: Vec<FpVar<F>>, // [merkle_root, recipient, prev_leaf_index_with_offset, prev_total_value]
        external_inputs: Self::ExternalInputsVar,
    ) -> Result<Vec<FpVar<F>>, SynthesisError> {
        let [
            merkle_root,
            recipient,
            prev_leaf_index_with_offset,
            prev_total_value,
        ]: [FpVar<F>; WITHDRAW_STATE_LEN] = z_i
            .try_into()
            .map_err(|_| SynthesisError::AssignmentMissing)?;

        let WithdrawExternalInputsVar {
            is_dummy,
            value,
            secret,
            leaf_index,
            siblings,
        } = external_inputs;
        let siblings: Vec<FpVar<F>> = siblings.into_iter().collect();

        let poseidon_params = CircomCRHParametersVar {
            parameters: self.poseidon_params.clone(),
        };

        let (out_root, out_recipient, out_leaf_index_with_offset, out_total) =
            withdraw_step::<F, DEPTH>(
                &poseidon_params,
                &merkle_root,
                &recipient,
                &prev_leaf_index_with_offset,
                &prev_total_value,
                &is_dummy,
                &value,
                &secret,
                &leaf_index,
                siblings.as_slice(),
            )?;

        Ok(vec![
            out_root,
            out_recipient,
            out_leaf_index_with_offset,
            out_total,
        ])
    }
}

pub fn dummy_withdraw_ext_input<const DEPTH: usize>(
    index: u64,
    value: U256,
) -> WithdrawExternalInputs<Fr, DEPTH> {
    WithdrawExternalInputs {
        is_dummy: Fr::ONE,
        value: u256_to_fr(value),
        secret: Fr::ZERO,
        leaf_index: Fr::from(index),
        siblings: core::array::from_fn(|_| Fr::ZERO),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        circuits::burn_address::{
            compute_burn_address_from_secret, find_pow_nonce, secret_from_nonce,
        },
        nova::params::NovaParams,
        utils::{
            convertion::fr_to_address, general_recipient::GeneralRecipient,
            poseidon::utils::circom_poseidon_config,
            tree::incremental_merkle_tree::IncrementalMerkleTree,
        },
    };
    use alloy::primitives::{Address, B256, U256};
    use ark_ff::AdditiveGroup;
    use folding_schemes::FoldingScheme;
    use rand::{SeedableRng, rngs::StdRng};
    use std::convert::TryInto;

    #[test]
    fn test_withdraw_circuit() {
        const DEPTH: usize = 4;
        let mut rng = StdRng::seed_from_u64(42);

        let recipient = GeneralRecipient {
            chain_id: 1,
            address: B256::left_padding_from(&[42]),
            tweak: B256::ZERO,
        }
        .to_fr();
        let secret_seeds = [123u64, 456, 789, 101112].map(Fr::from);
        let secrets_and_addresses = secret_seeds
            .iter()
            .map(|seed| {
                let nonce = find_pow_nonce(recipient, *seed);
                let secret = secret_from_nonce(*seed, nonce);
                let address = compute_burn_address_from_secret(recipient, secret)
                    .expect("nonce should satisfy PoW");
                (secret, address)
            })
            .collect::<Vec<_>>();
        let secrets = secrets_and_addresses
            .iter()
            .map(|(secret, _)| *secret)
            .collect::<Vec<_>>();
        let addresses = secrets_and_addresses
            .iter()
            .map(|(_, address)| *address)
            .collect::<Vec<_>>();
        let values = vec![
            U256::from(1000u64),
            U256::from(2000u64),
            U256::from(3000u64),
            U256::from(4000u64),
        ];

        let mut tree = IncrementalMerkleTree::new(DEPTH);
        tree.insert(Address::ZERO, U256::ZERO);

        let mut indices = vec![];
        for i in 0..4 {
            let index = tree.insert(fr_to_address(addresses[i]), values[i]);
            indices.push(index);
        }
        let root = tree.get_root();

        let z_0 = vec![root, recipient, Fr::ZERO, Fr::ZERO];

        let mut external_inputs = vec![];
        for (i, value) in values.iter().enumerate() {
            let leaf_index = indices[i];
            let proof = tree.prove(leaf_index);
            let ext_input = WithdrawExternalInputs::<Fr, DEPTH> {
                is_dummy: Fr::ZERO,
                value: u256_to_fr(*value),
                secret: secrets[i],
                leaf_index: Fr::from(leaf_index),
                siblings: proof.siblings.try_into().unwrap(),
            };
            external_inputs.push(ext_input);
        }

        for i in 0..4 {
            let ext_input = dummy_withdraw_ext_input(5 + i as u64, U256::ZERO);
            external_inputs.push(ext_input);
        }

        let f_params = circom_poseidon_config::<Fr>();
        let nova_params =
            NovaParams::<WithdrawCircuit<Fr, DEPTH>>::rand(f_params, &mut rng).unwrap();

        let mut nova = nova_params.initial_nova(z_0.clone()).unwrap();
        for external_input in external_inputs.iter() {
            nova.prove_step(&mut rng, external_input.clone(), None)
                .unwrap();
        }
        let ivc_proof = nova.ivc_proof();
        nova_params.verify(ivc_proof).unwrap();
    }
}
