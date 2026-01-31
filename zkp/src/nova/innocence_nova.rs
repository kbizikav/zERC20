//! Nova IVC wrapper for Proof of Innocence circuit.
//!
//! This module provides the `InnocenceCircuit` struct that implements the `FCircuit`
//! trait from the `folding_schemes` crate, enabling Nova incremental verification
//! for batched OFAC non-membership proofs.
//!
//! # State Vector
//!
//! The circuit maintains a state vector of 3 elements:
//! - `ofac_root`: Root of the OFAC exclusion tree (constant across all steps)
//! - `recipient`: Hash of the GeneralRecipient (constant across all steps)
//! - `total_teleported`: Running sum of transfer values (accumulator)
//!
//! # Usage
//!
//! ```ignore
//! let circuit = InnocenceCircuit::new(poseidon2_params)?;
//! let z_0 = vec![ofac_root, recipient, Fr::zero()];
//! let mut nova = nova_params.initial_nova(z_0)?;
//!
//! for transfer in transfers {
//!     nova.prove_step(&mut rng, transfer.to_external_inputs(), None)?;
//! }
//!
//! let proof = nova.ivc_proof();
//! ```

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

use crate::{
    circuits::proof_of_innocence::innocence_step,
    utils::poseidon::gadgets::CircomCRHParametersVar,
};

/// State vector length: [ofac_root, recipient, total_teleported]
pub const INNOCENCE_STATE_LEN: usize = 3;

/// Nova circuit wrapper for Proof of Innocence.
///
/// This circuit implements the `FCircuit` trait, allowing it to be used with
/// Nova's incremental verification computation (IVC) for efficient batched proofs.
#[derive(Clone, Debug)]
pub struct InnocenceCircuit<F: PrimeField + Absorb, const DEPTH: usize> {
    /// Poseidon parameters for 2-to-1 hashing (gap leaves and Merkle tree)
    pub poseidon2_params: PoseidonConfig<F>,
}

/// External inputs for each step of the Proof of Innocence circuit.
///
/// These are the per-step private inputs that vary for each transfer being proven.
#[derive(Clone, Debug)]
pub struct InnocenceExternalInputs<F: PrimeField, const DEPTH: usize> {
    /// Whether this is a padding/dummy step (skips verification)
    pub is_dummy: bool,
    /// Sender address to prove is not sanctioned
    pub from_address: F,
    /// Transfer value
    pub value: F,
    /// Lower bound of the exclusion gap containing `from_address`
    pub start: F,
    /// Upper bound of the exclusion gap containing `from_address`
    pub end: F,
    /// Position of this gap leaf in the exclusion tree
    pub gap_index: F,
    /// Merkle proof siblings for the gap leaf
    pub siblings: [F; DEPTH],
}

impl<F: PrimeField, const DEPTH: usize> Default for InnocenceExternalInputs<F, DEPTH> {
    fn default() -> Self {
        Self {
            is_dummy: false,
            from_address: F::zero(),
            value: F::zero(),
            start: F::zero(),
            end: F::zero(),
            gap_index: F::zero(),
            siblings: core::array::from_fn(|_| F::zero()),
        }
    }
}

/// Circuit variable version of `InnocenceExternalInputs`.
#[derive(Clone, Debug)]
pub struct InnocenceExternalInputsVar<F: PrimeField, const DEPTH: usize> {
    pub is_dummy: Boolean<F>,
    pub from_address: FpVar<F>,
    pub value: FpVar<F>,
    pub start: FpVar<F>,
    pub end: FpVar<F>,
    pub gap_index: FpVar<F>,
    pub siblings: [FpVar<F>; DEPTH],
}

impl<F: PrimeField, const DEPTH: usize> AllocVar<InnocenceExternalInputs<F, DEPTH>, F>
    for InnocenceExternalInputsVar<F, DEPTH>
{
    fn new_variable<T: Borrow<InnocenceExternalInputs<F, DEPTH>>>(
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
            let val = FpVar::<F>::new_variable(cs.clone(), || Ok(value.value), mode)?;
            let start = FpVar::<F>::new_variable(cs.clone(), || Ok(value.start), mode)?;
            let end = FpVar::<F>::new_variable(cs.clone(), || Ok(value.end), mode)?;
            let gap_index = FpVar::<F>::new_variable(cs.clone(), || Ok(value.gap_index), mode)?;
            let siblings = <[FpVar<F>; DEPTH] as AllocVar<[F; DEPTH], F>>::new_variable(
                cs,
                || Ok(value.siblings),
                mode,
            )?;
            Ok(Self {
                is_dummy,
                from_address,
                value: val,
                start,
                end,
                gap_index,
                siblings,
            })
        })
    }
}

impl<F: PrimeField + Absorb, const DEPTH: usize> FCircuit<F> for InnocenceCircuit<F, DEPTH> {
    type Params = PoseidonConfig<F>;
    type ExternalInputs = InnocenceExternalInputs<F, DEPTH>;
    type ExternalInputsVar = InnocenceExternalInputsVar<F, DEPTH>;

    fn new(params: Self::Params) -> Result<Self, Error> {
        Ok(Self {
            poseidon2_params: params,
        })
    }

    fn state_len(&self) -> usize {
        INNOCENCE_STATE_LEN
    }

    fn generate_step_constraints(
        &self,
        _cs: ConstraintSystemRef<F>,
        _i: usize,
        z_i: Vec<FpVar<F>>, // [ofac_root, recipient, prev_total_teleported]
        external_inputs: Self::ExternalInputsVar,
    ) -> Result<Vec<FpVar<F>>, SynthesisError> {
        let [ofac_root, recipient, prev_total_teleported]: [FpVar<F>; INNOCENCE_STATE_LEN] = z_i
            .try_into()
            .map_err(|_| SynthesisError::AssignmentMissing)?;

        let InnocenceExternalInputsVar {
            is_dummy,
            from_address,
            value,
            start,
            end,
            gap_index,
            siblings,
        } = external_inputs;
        let siblings: Vec<FpVar<F>> = siblings.into_iter().collect();

        let poseidon2_params = CircomCRHParametersVar {
            parameters: self.poseidon2_params.clone(),
        };

        let new_total_teleported = innocence_step::<F, DEPTH>(
            &poseidon2_params,
            &ofac_root,
            &from_address,
            &prev_total_teleported,
            &is_dummy,
            &value,
            &start,
            &end,
            &gap_index,
            siblings.as_slice(),
        )?;

        // ofac_root and recipient are constant across all steps
        Ok(vec![ofac_root, recipient, new_total_teleported])
    }
}

/// Creates a dummy external input for batch padding.
///
/// Dummy steps subtract their value from the total, so to achieve net-zero effect,
/// pair each dummy with `value = 0` or ensure dummy values cancel out.
pub fn dummy_innocence_ext_input<F: PrimeField, const DEPTH: usize>(
    value: F,
) -> InnocenceExternalInputs<F, DEPTH> {
    InnocenceExternalInputs {
        is_dummy: true,
        from_address: F::zero(),
        value,
        start: F::zero(),
        end: F::zero(),
        gap_index: F::zero(),
        siblings: core::array::from_fn(|_| F::zero()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        nova::params::NovaParams,
        utils::poseidon::utils::{circom_poseidon2_config, poseidon2},
    };
    use ark_bn254::Fr;
    use ark_ff::AdditiveGroup;
    use folding_schemes::FoldingScheme;
    use rand::{rngs::StdRng, SeedableRng};

    const DEPTH: usize = 4;

    /// Helper to build a simple exclusion tree for testing.
    fn build_test_exclusion_tree(gaps: &[(Fr, Fr)]) -> Fr {
        let leaves: Vec<Fr> = gaps.iter().map(|(s, e)| poseidon2(*s, *e)).collect();

        // Pad to power of 2
        let mut padded_leaves = leaves.clone();
        while padded_leaves.len() < (1 << DEPTH) {
            padded_leaves.push(Fr::from(0u64));
        }

        // Build tree bottom-up
        let mut current_level = padded_leaves;
        while current_level.len() > 1 {
            let mut next_level = Vec::new();
            for chunk in current_level.chunks(2) {
                let hash = poseidon2(chunk[0], chunk[1]);
                next_level.push(hash);
            }
            current_level = next_level;
        }
        current_level[0]
    }

    /// Helper to get Merkle proof for a leaf at given index
    fn get_merkle_proof(gaps: &[(Fr, Fr)], index: usize) -> [Fr; DEPTH] {
        let leaves: Vec<Fr> = gaps.iter().map(|(s, e)| poseidon2(*s, *e)).collect();

        // Pad to power of 2
        let mut padded_leaves = leaves.clone();
        while padded_leaves.len() < (1 << DEPTH) {
            padded_leaves.push(Fr::from(0u64));
        }

        let mut siblings = Vec::new();
        let mut current_level = padded_leaves;
        let mut current_index = index;

        for _ in 0..DEPTH {
            let sibling_index = if current_index % 2 == 0 {
                current_index + 1
            } else {
                current_index - 1
            };
            siblings.push(current_level[sibling_index]);

            // Build next level
            let mut next_level = Vec::new();
            for chunk in current_level.chunks(2) {
                let hash = poseidon2(chunk[0], chunk[1]);
                next_level.push(hash);
            }
            current_level = next_level;
            current_index /= 2;
        }

        siblings.try_into().unwrap()
    }

    #[test]
    fn test_innocence_circuit_single_step() {
        let mut rng = StdRng::seed_from_u64(42);

        // Create exclusion tree with 3 gaps
        let gaps = vec![
            (Fr::from(0u64), Fr::from(100u64)),
            (Fr::from(100u64), Fr::from(200u64)),
            (Fr::from(200u64), Fr::from(1000u64)),
        ];
        let ofac_root = build_test_exclusion_tree(&gaps);
        let recipient = Fr::from(12345u64);

        // Initial state: [ofac_root, recipient, 0]
        let z_0 = vec![ofac_root, recipient, Fr::ZERO];

        // Create external input for a valid transfer
        let ext_input = InnocenceExternalInputs::<Fr, DEPTH> {
            is_dummy: false,
            from_address: Fr::from(150u64), // In gap 1: (100, 200)
            value: Fr::from(1000u64),
            start: Fr::from(100u64),
            end: Fr::from(200u64),
            gap_index: Fr::from(1u64),
            siblings: get_merkle_proof(&gaps, 1),
        };

        let poseidon2_params = circom_poseidon2_config::<Fr>();
        let nova_params = NovaParams::<InnocenceCircuit<Fr, DEPTH>>::rand(
            poseidon2_params,
            &mut rng,
        )
        .unwrap();

        let mut nova = nova_params.initial_nova(z_0.clone()).unwrap();
        nova.prove_step(&mut rng, ext_input, None).unwrap();

        let ivc_proof = nova.ivc_proof();
        nova_params.verify(ivc_proof).unwrap();
    }

    #[test]
    fn test_innocence_circuit_multiple_steps() {
        let mut rng = StdRng::seed_from_u64(42);

        // Create exclusion tree
        let gaps = vec![
            (Fr::from(0u64), Fr::from(100u64)),
            (Fr::from(100u64), Fr::from(200u64)),
            (Fr::from(200u64), Fr::from(1000u64)),
        ];
        let ofac_root = build_test_exclusion_tree(&gaps);
        let recipient = Fr::from(12345u64);

        let z_0 = vec![ofac_root, recipient, Fr::ZERO];

        // Multiple transfers from different non-sanctioned addresses
        let transfers = vec![
            // Address 50 is in gap 0: (0, 100)
            InnocenceExternalInputs::<Fr, DEPTH> {
                is_dummy: false,
                from_address: Fr::from(50u64),
                value: Fr::from(100u64),
                start: Fr::from(0u64),
                end: Fr::from(100u64),
                gap_index: Fr::from(0u64),
                siblings: get_merkle_proof(&gaps, 0),
            },
            // Address 150 is in gap 1: (100, 200)
            InnocenceExternalInputs::<Fr, DEPTH> {
                is_dummy: false,
                from_address: Fr::from(150u64),
                value: Fr::from(200u64),
                start: Fr::from(100u64),
                end: Fr::from(200u64),
                gap_index: Fr::from(1u64),
                siblings: get_merkle_proof(&gaps, 1),
            },
            // Address 500 is in gap 2: (200, 1000)
            InnocenceExternalInputs::<Fr, DEPTH> {
                is_dummy: false,
                from_address: Fr::from(500u64),
                value: Fr::from(300u64),
                start: Fr::from(200u64),
                end: Fr::from(1000u64),
                gap_index: Fr::from(2u64),
                siblings: get_merkle_proof(&gaps, 2),
            },
        ];

        let poseidon2_params = circom_poseidon2_config::<Fr>();
        let nova_params = NovaParams::<InnocenceCircuit<Fr, DEPTH>>::rand(
            poseidon2_params,
            &mut rng,
        )
        .unwrap();

        let mut nova = nova_params.initial_nova(z_0).unwrap();

        for ext_input in transfers {
            nova.prove_step(&mut rng, ext_input, None).unwrap();
        }

        // Add dummy steps to pad the batch
        for _ in 0..5 {
            let dummy = dummy_innocence_ext_input::<Fr, DEPTH>(Fr::ZERO);
            nova.prove_step(&mut rng, dummy, None).unwrap();
        }

        let ivc_proof = nova.ivc_proof();
        nova_params.verify(ivc_proof).unwrap();

        // Verify final state
        // Total should be 100 + 200 + 300 = 600
        let final_state = nova.state();
        assert_eq!(final_state[0], ofac_root); // ofac_root unchanged
        assert_eq!(final_state[1], recipient); // recipient unchanged
        assert_eq!(final_state[2], Fr::from(600u64)); // total teleported
    }

    #[test]
    fn test_innocence_circuit_with_dummy_padding() {
        let mut rng = StdRng::seed_from_u64(42);

        let gaps = vec![
            (Fr::from(0u64), Fr::from(100u64)),
            (Fr::from(100u64), Fr::from(200u64)),
        ];
        let ofac_root = build_test_exclusion_tree(&gaps);
        let recipient = Fr::from(999u64);

        let z_0 = vec![ofac_root, recipient, Fr::ZERO];

        // One real transfer
        let real_transfer = InnocenceExternalInputs::<Fr, DEPTH> {
            is_dummy: false,
            from_address: Fr::from(50u64),
            value: Fr::from(1000u64),
            start: Fr::from(0u64),
            end: Fr::from(100u64),
            gap_index: Fr::from(0u64),
            siblings: get_merkle_proof(&gaps, 0),
        };

        let poseidon2_params = circom_poseidon2_config::<Fr>();
        let nova_params = NovaParams::<InnocenceCircuit<Fr, DEPTH>>::rand(
            poseidon2_params,
            &mut rng,
        )
        .unwrap();

        let mut nova = nova_params.initial_nova(z_0).unwrap();

        // Real step
        nova.prove_step(&mut rng, real_transfer, None).unwrap();

        // Dummy steps with zero value (no effect on total)
        for _ in 0..3 {
            let dummy = dummy_innocence_ext_input::<Fr, DEPTH>(Fr::ZERO);
            nova.prove_step(&mut rng, dummy, None).unwrap();
        }

        let ivc_proof = nova.ivc_proof();
        nova_params.verify(ivc_proof).unwrap();

        // Total should still be 1000 (dummy steps with value=0 don't change it)
        let final_state = nova.state();
        assert_eq!(final_state[2], Fr::from(1000u64));
    }
}
