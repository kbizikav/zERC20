use crate::{
    circuits::root_transition::root_transition_step, nova::constants::TRANSFER_TREE_HEIGHT,
    utils::poseidon::gadgets::CircomCRHParametersVar,
};
use ark_crypto_primitives::sponge::{Absorb, poseidon::PoseidonConfig};
use ark_ff::PrimeField;
use ark_r1cs_std::{
    alloc::{AllocVar, AllocationMode},
    boolean::Boolean,
    fields::fp::FpVar,
};
use ark_relations::gr1cs::{ConstraintSystemRef, Namespace, SynthesisError};
use ark_std::vec::Vec;
use core::{borrow::Borrow, convert::TryInto};
use folding_schemes::{Error, frontend::FCircuit};

const ROOT_STATE_LEN: usize = 3;

#[derive(Clone, Debug)]
pub struct RootCircuit<F: PrimeField + Absorb> {
    pub poseidon2_params: PoseidonConfig<F>,
    pub poseidon3_params: PoseidonConfig<F>,
}

#[derive(Clone, Debug)]
pub struct RootExternalInputs<F: PrimeField> {
    pub is_dummy: bool,
    pub from_address: F,
    pub to_address: F,
    pub value: F,
    pub siblings: [F; TRANSFER_TREE_HEIGHT],
}

impl<F: PrimeField> Default for RootExternalInputs<F> {
    fn default() -> Self {
        Self {
            is_dummy: false,
            from_address: F::zero(),
            to_address: F::zero(),
            value: F::zero(),
            siblings: core::array::from_fn(|_| F::zero()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RootExternalInputsVar<F: PrimeField> {
    pub is_dummy: Boolean<F>,
    pub from_address: FpVar<F>,
    pub to_address: FpVar<F>,
    pub value: FpVar<F>,
    pub siblings: [FpVar<F>; TRANSFER_TREE_HEIGHT],
}

impl<F: PrimeField> AllocVar<RootExternalInputs<F>, F> for RootExternalInputsVar<F> {
    fn new_variable<T: Borrow<RootExternalInputs<F>>>(
        cs: impl Into<Namespace<F>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
        mode: AllocationMode,
    ) -> Result<Self, SynthesisError> {
        let ns = cs.into();
        let cs = ns.cs();
        f().and_then(|value| {
            let value = value.borrow();
            let is_dummy = Boolean::new_variable(cs.clone(), || Ok(value.is_dummy), mode)?;
            let from_address =
                FpVar::<F>::new_variable(cs.clone(), || Ok(value.from_address), mode)?;
            let to_address = FpVar::<F>::new_variable(cs.clone(), || Ok(value.to_address), mode)?;
            let val = FpVar::<F>::new_variable(cs.clone(), || Ok(value.value), mode)?;
            let siblings = <[FpVar<F>; TRANSFER_TREE_HEIGHT] as AllocVar<
                [F; TRANSFER_TREE_HEIGHT],
                F,
            >>::new_variable(
                cs.clone(), || Ok(value.siblings), mode
            )?;
            Ok(Self {
                is_dummy,
                from_address,
                to_address,
                value: val,
                siblings,
            })
        })
    }
}

impl<F: PrimeField + Absorb> FCircuit<F> for RootCircuit<F> {
    type Params = (PoseidonConfig<F>, PoseidonConfig<F>);
    // External inputs layout: [from_address, to_address, value, sibling_0, ..., sibling_{DEPTH-1}]
    type ExternalInputs = RootExternalInputs<F>;
    type ExternalInputsVar = RootExternalInputsVar<F>;

    fn new(params: Self::Params) -> Result<Self, Error> {
        let (poseidon2_params, poseidon3_params) = params;
        Ok(Self {
            poseidon2_params,
            poseidon3_params,
        })
    }

    fn state_len(&self) -> usize {
        ROOT_STATE_LEN // [index, hash_chain, merkle_root]
    }

    fn generate_step_constraints(
        &self,
        _cs: ConstraintSystemRef<F>,
        _i: usize,
        z_i: Vec<FpVar<F>>, // [index, hash_chain, merkle_root]
        external_inputs: Self::ExternalInputsVar, // [address, value, siblings..]
    ) -> Result<Vec<FpVar<F>>, SynthesisError> {
        let [index, prev_hash_chain, prev_root]: [FpVar<F>; ROOT_STATE_LEN] = z_i
            .try_into()
            .map_err(|_| SynthesisError::AssignmentMissing)?;
        let RootExternalInputsVar {
            is_dummy,
            from_address,
            to_address,
            value,
            siblings,
        } = external_inputs;
        let siblings: Vec<FpVar<F>> = siblings.into_iter().collect();

        let poseidon2_params = CircomCRHParametersVar {
            parameters: self.poseidon2_params.clone(),
        };
        let poseidon3_params = CircomCRHParametersVar {
            parameters: self.poseidon3_params.clone(),
        };
        let (new_index, new_hash_chain, new_root) = root_transition_step::<F, TRANSFER_TREE_HEIGHT>(
            &poseidon2_params,
            &poseidon3_params,
            &index,
            &prev_hash_chain,
            &prev_root,
            &from_address,
            &to_address,
            &value,
            siblings.as_slice(),
            &is_dummy,
        )?;
        Ok(vec![new_index, new_hash_chain, new_root])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        nova::params::NovaParams,
        utils::{
            convertion::{address_to_fr, u256_to_fr},
            poseidon::utils::{circom_poseidon2_config, circom_poseidon3_config},
            tree::incremental_merkle_tree::{IncrementalMerkleTree, Leaf},
        },
    };
    use alloy::primitives::{Address, U256};
    use ark_bn254::Fr;
    use ark_ff::AdditiveGroup;
    use core::convert::TryInto;
    use folding_schemes::FoldingScheme;
    use rand::{RngCore, SeedableRng, rngs::StdRng};

    #[test]
    fn test_root_circuit() {
        let mut rng = StdRng::seed_from_u64(42);

        let mut tree = IncrementalMerkleTree::new(TRANSFER_TREE_HEIGHT);
        let z_0 = vec![
            Fr::from(tree.index),
            u256_to_fr(tree.hash_chain),
            tree.get_root(),
        ];

        let mut external_inputs = vec![];
        for i in 0..4 {
            let from_address = Address::left_padding_from(&[(i + 1) as u8]);
            let to_address = Address::left_padding_from(&[i as u8]);
            let value = U256::from(rng.next_u64());

            let index = tree.index;
            let proof = tree.prove(index);
            let calculated_root = proof.get_root(Fr::ZERO, index);
            assert_eq!(calculated_root, tree.get_root());

            tree.insert(from_address, to_address, value)
                .expect("test tree insert within capacity");
            let leaf_hash = Leaf {
                from: from_address,
                to: to_address,
                value,
            }
            .hash();
            let calculated_root = proof.get_root(leaf_hash, index);
            assert_eq!(calculated_root, tree.get_root());

            let siblings: [Fr; TRANSFER_TREE_HEIGHT] = proof
                .siblings
                .clone()
                .try_into()
                .expect("sibling path length");
            external_inputs.push(RootExternalInputs::<Fr> {
                is_dummy: false,
                from_address: address_to_fr(from_address),
                to_address: address_to_fr(to_address),
                value: u256_to_fr(value),
                siblings,
            });
        }

        let expected_index = tree.index;
        let expected_hash_chain = tree.hash_chain;
        let expected_root = tree.get_root();

        let poseidon2_config = circom_poseidon2_config::<Fr>();
        let poseidon3_config = circom_poseidon3_config();
        let nova_params =
            NovaParams::<RootCircuit<Fr>>::rand((poseidon2_config, poseidon3_config), &mut rng)
                .unwrap();

        let mut nova = nova_params.initial_nova(z_0.clone()).unwrap();

        for external_input in external_inputs.iter() {
            nova.prove_step(&mut rng, external_input.clone(), None)
                .unwrap();
        }

        let state = nova.state();
        assert_eq!(state[0], Fr::from(expected_index));
        assert_eq!(state[1], u256_to_fr(expected_hash_chain));
        assert_eq!(state[2], expected_root);

        let ivc_proof = nova.ivc_proof();
        nova_params.verify(ivc_proof).unwrap();
    }
}
