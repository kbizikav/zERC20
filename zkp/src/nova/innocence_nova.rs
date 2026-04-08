// SPDX-License-Identifier: BUSL-1.1

//! Nova IVC wrapper for Proof of Innocence circuit.
//!
//! This module provides the `InnocenceCircuit` struct that implements the `FCircuit`
//! trait from the `folding_schemes` crate, enabling Nova incremental verification
//! for batched OFAC non-membership proofs.
//!
//! # State Vector
//!
//! The circuit maintains a state vector of 5 elements:
//! - `ofac_root`: Root of the OFAC exclusion tree (constant across all steps)
//! - `recipient`: Hash of the GeneralRecipient (constant across all steps)
//! - `merkle_root`: Root of the transfer tree (constant across all steps)
//! - `leaf_index_with_offset`: Monotonically increasing leaf index tracker (anti-replay)
//! - `total_teleported`: Running sum of transfer values (accumulator)
//!
//! # Recipient Binding
//!
//! Each transfer is bound to the recipient via burn address PoW. The prover must
//! provide a `secret` that satisfies `derive(recipient, secret)` with PoW constraint.
//! This prevents attackers from using another user's transfers in their proof.
//!
//! # Transfer Inclusion & Anti-Replay
//!
//! Each step verifies a Merkle inclusion proof for the transfer leaf in the transfer tree
//! and enforces strictly monotonically increasing leaf indices to prevent replay attacks.
//!
//! # Usage
//!
//! ```ignore
//! let params = (poseidon2_config, poseidon3_config);
//! let circuit = InnocenceCircuit::new(params)?;
//! let z_0 = vec![ofac_root, recipient, transfer_tree_root, Fr::zero(), Fr::zero()];
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
    circuits::proof_of_innocence::innocence_step, utils::poseidon::gadgets::CircomCRHParametersVar,
};

/// State vector length: [ofac_root, recipient, merkle_root, leaf_index_with_offset, total_teleported]
pub const INNOCENCE_STATE_LEN: usize = 5;

/// Nova circuit wrapper for Proof of Innocence.
///
/// This circuit implements the `FCircuit` trait, allowing it to be used with
/// Nova's incremental verification computation (IVC) for efficient batched proofs.
#[derive(Clone, Debug)]
pub struct InnocenceCircuit<F: PrimeField + Absorb, const DEPTH: usize, const TRANSFER_DEPTH: usize>
{
    /// Poseidon parameters for 2-to-1 hashing (gap leaves and Merkle tree)
    pub poseidon2_params: PoseidonConfig<F>,
    /// Poseidon parameters for 3-to-1 hashing (burn address derivation and leaf hash)
    pub poseidon3_params: PoseidonConfig<F>,
}

/// External inputs for each step of the Proof of Innocence circuit.
///
/// These are the per-step private inputs that vary for each transfer being proven.
#[derive(Clone, Debug)]
pub struct InnocenceExternalInputs<F: PrimeField, const DEPTH: usize, const TRANSFER_DEPTH: usize> {
    /// Whether this is a padding/dummy step (skips verification)
    pub is_dummy: bool,
    /// Sender address to prove is not sanctioned
    pub from_address: F,
    /// Transfer value
    pub value: F,
    /// Secret used to derive burn address (proves transfer belongs to recipient)
    pub secret: F,
    /// Position of this transfer in the transfer tree
    pub leaf_index: F,
    /// Merkle proof siblings for the transfer leaf
    pub transfer_siblings: [F; TRANSFER_DEPTH],
    /// Lower bound of the exclusion gap containing `from_address`
    pub start: F,
    /// Upper bound of the exclusion gap containing `from_address`
    pub end: F,
    /// Position of this gap leaf in the exclusion tree
    pub gap_index: F,
    /// Merkle proof siblings for the gap leaf
    pub siblings: [F; DEPTH],
}

impl<F: PrimeField, const DEPTH: usize, const TRANSFER_DEPTH: usize> Default
    for InnocenceExternalInputs<F, DEPTH, TRANSFER_DEPTH>
{
    fn default() -> Self {
        Self {
            is_dummy: false,
            from_address: F::zero(),
            value: F::zero(),
            secret: F::zero(),
            leaf_index: F::zero(),
            transfer_siblings: core::array::from_fn(|_| F::zero()),
            start: F::zero(),
            end: F::zero(),
            gap_index: F::zero(),
            siblings: core::array::from_fn(|_| F::zero()),
        }
    }
}

/// Circuit variable version of `InnocenceExternalInputs`.
#[derive(Clone, Debug)]
pub struct InnocenceExternalInputsVar<
    F: PrimeField,
    const DEPTH: usize,
    const TRANSFER_DEPTH: usize,
> {
    pub is_dummy: Boolean<F>,
    pub from_address: FpVar<F>,
    pub value: FpVar<F>,
    pub secret: FpVar<F>,
    pub leaf_index: FpVar<F>,
    pub transfer_siblings: [FpVar<F>; TRANSFER_DEPTH],
    pub start: FpVar<F>,
    pub end: FpVar<F>,
    pub gap_index: FpVar<F>,
    pub siblings: [FpVar<F>; DEPTH],
}

impl<F: PrimeField, const DEPTH: usize, const TRANSFER_DEPTH: usize>
    AllocVar<InnocenceExternalInputs<F, DEPTH, TRANSFER_DEPTH>, F>
    for InnocenceExternalInputsVar<F, DEPTH, TRANSFER_DEPTH>
{
    fn new_variable<T: Borrow<InnocenceExternalInputs<F, DEPTH, TRANSFER_DEPTH>>>(
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
            let secret = FpVar::<F>::new_variable(cs.clone(), || Ok(value.secret), mode)?;
            let leaf_index = FpVar::<F>::new_variable(cs.clone(), || Ok(value.leaf_index), mode)?;
            let transfer_siblings = <[FpVar<F>; TRANSFER_DEPTH] as AllocVar<
                [F; TRANSFER_DEPTH],
                F,
            >>::new_variable(
                cs.clone(), || Ok(value.transfer_siblings), mode
            )?;
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
                secret,
                leaf_index,
                transfer_siblings,
                start,
                end,
                gap_index,
                siblings,
            })
        })
    }
}

impl<F: PrimeField + Absorb, const DEPTH: usize, const TRANSFER_DEPTH: usize> FCircuit<F>
    for InnocenceCircuit<F, DEPTH, TRANSFER_DEPTH>
{
    type Params = (PoseidonConfig<F>, PoseidonConfig<F>);
    type ExternalInputs = InnocenceExternalInputs<F, DEPTH, TRANSFER_DEPTH>;
    type ExternalInputsVar = InnocenceExternalInputsVar<F, DEPTH, TRANSFER_DEPTH>;

    fn new(params: Self::Params) -> Result<Self, Error> {
        let (poseidon2_params, poseidon3_params) = params;
        Ok(Self {
            poseidon2_params,
            poseidon3_params,
        })
    }

    fn state_len(&self) -> usize {
        INNOCENCE_STATE_LEN
    }

    fn generate_step_constraints(
        &self,
        _cs: ConstraintSystemRef<F>,
        _i: usize,
        z_i: Vec<FpVar<F>>, // [ofac_root, recipient, merkle_root, prev_leaf_index_with_offset, prev_total_teleported]
        external_inputs: Self::ExternalInputsVar,
    ) -> Result<Vec<FpVar<F>>, SynthesisError> {
        let [
            ofac_root,
            recipient,
            merkle_root,
            prev_leaf_index_with_offset,
            prev_total_teleported,
        ]: [FpVar<F>; INNOCENCE_STATE_LEN] = z_i
            .try_into()
            .map_err(|_| SynthesisError::AssignmentMissing)?;

        let InnocenceExternalInputsVar {
            is_dummy,
            from_address,
            value,
            secret,
            leaf_index,
            transfer_siblings,
            start,
            end,
            gap_index,
            siblings,
        } = external_inputs;
        let siblings: Vec<FpVar<F>> = siblings.into_iter().collect();
        let transfer_siblings: Vec<FpVar<F>> = transfer_siblings.into_iter().collect();

        let poseidon2_params = CircomCRHParametersVar {
            parameters: self.poseidon2_params.clone(),
        };
        let poseidon3_params = CircomCRHParametersVar {
            parameters: self.poseidon3_params.clone(),
        };

        let (leaf_index_with_offset, new_total_teleported) =
            innocence_step::<F, DEPTH, TRANSFER_DEPTH>(
                &poseidon2_params,
                &poseidon3_params,
                &ofac_root,
                &recipient,
                &merkle_root,
                &from_address,
                &prev_leaf_index_with_offset,
                &prev_total_teleported,
                &is_dummy,
                &value,
                &secret,
                &leaf_index,
                transfer_siblings.as_slice(),
                &start,
                &end,
                &gap_index,
                siblings.as_slice(),
            )?;

        // ofac_root, recipient, and merkle_root are constant across all steps
        Ok(vec![
            ofac_root,
            recipient,
            merkle_root,
            leaf_index_with_offset,
            new_total_teleported,
        ])
    }
}

/// Creates a dummy external input for batch padding.
///
/// Dummy steps subtract their value from the total, so to achieve net-zero effect,
/// pair each dummy with `value = 0` or ensure dummy values cancel out.
///
/// The `index` parameter provides a monotonically increasing leaf index for the dummy step.
pub fn dummy_innocence_ext_input<F: PrimeField, const DEPTH: usize, const TRANSFER_DEPTH: usize>(
    index: u64,
    value: F,
) -> InnocenceExternalInputs<F, DEPTH, TRANSFER_DEPTH> {
    InnocenceExternalInputs {
        is_dummy: true,
        from_address: F::zero(),
        value,
        secret: F::zero(),
        leaf_index: F::from(index),
        transfer_siblings: core::array::from_fn(|_| F::zero()),
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
        circuits::burn_address::{
            compute_burn_address_from_secret, find_pow_nonce, secret_from_nonce,
        },
        nova::params::NovaParams,
        utils::{
            convertion::{fr_to_address, fr_to_u256},
            exclusion_tree::ExclusionTree,
            poseidon::utils::{circom_poseidon2_config, circom_poseidon3_config},
            tree::incremental_merkle_tree::IncrementalMerkleTree,
        },
    };
    use alloy::primitives::U256;
    use ark_bn254::Fr;
    use ark_ff::AdditiveGroup;
    use folding_schemes::FoldingScheme;
    use rand::{SeedableRng, rngs::StdRng};

    const OFAC_LIST: &str = include_str!("../../data/ofac_sanction_list.txt");
    // 81 sanctioned addresses -> 82 gaps -> need 2^7 = 128 leaves
    const DEPTH: usize = 7;
    const TRANSFER_DEPTH: usize = 4;

    fn build_ofac_tree() -> ExclusionTree {
        let lines: Vec<&str> = OFAC_LIST.lines().collect();
        let addresses = ExclusionTree::parse_addresses(&lines);
        ExclusionTree::from_sorted_addresses(&addresses, DEPTH)
    }

    /// Helper to find a valid secret for a recipient (satisfies PoW)
    fn find_valid_secret(recipient: Fr, seed: u64) -> Fr {
        let secret_seed = Fr::from(seed);
        let nonce = find_pow_nonce(recipient, secret_seed);
        secret_from_nonce(secret_seed, nonce)
    }

    // Innocent addresses falling in different gaps of the OFAC exclusion tree
    const INNOCENT_ADDR_1: &str = "0x0100000000000000000000000000000000000000";
    const INNOCENT_ADDR_2: &str = "0x0600000000000000000000000000000000000000";
    const INNOCENT_ADDR_3: &str = "0x1000000000000000000000000000000000000000";

    #[test]
    #[ignore = "Nova IVC proof is too slow for CI in debug mode (~113s)"]
    fn test_innocence_circuit_single_step() {
        let mut rng = StdRng::seed_from_u64(42);

        let tree = build_ofac_tree();
        let ofac_root = tree.root();
        let recipient = Fr::from(12345u64);
        let secret = find_valid_secret(recipient, 1000);
        let burn_address = compute_burn_address_from_secret(recipient, secret).expect("valid PoW");

        let from_address = ExclusionTree::parse_addresses(&[INNOCENT_ADDR_1])[0];
        let proof = tree
            .prove_non_membership(from_address)
            .expect("address should not be sanctioned");

        let value = Fr::from(1000u64);

        // Build transfer tree
        let mut transfer_tree = IncrementalMerkleTree::new(TRANSFER_DEPTH);
        let leaf_index = transfer_tree
            .insert(
                fr_to_address(from_address),
                fr_to_address(burn_address),
                fr_to_u256(value),
            )
            .expect("insert should succeed");
        let transfer_root = transfer_tree.get_root();
        let transfer_proof = transfer_tree.prove(leaf_index);

        let z_0 = vec![ofac_root, recipient, transfer_root, Fr::ZERO, Fr::ZERO];

        let ext_input = InnocenceExternalInputs::<Fr, DEPTH, TRANSFER_DEPTH> {
            is_dummy: false,
            from_address,
            value,
            secret,
            leaf_index: Fr::from(leaf_index),
            transfer_siblings: transfer_proof.siblings.try_into().unwrap(),
            start: proof.start,
            end: proof.end,
            gap_index: Fr::from(proof.gap_index),
            siblings: proof.siblings_array(),
        };

        let poseidon2_params = circom_poseidon2_config::<Fr>();
        let poseidon3_params = circom_poseidon3_config::<Fr>();
        let nova_params = NovaParams::<InnocenceCircuit<Fr, DEPTH, TRANSFER_DEPTH>>::rand(
            (poseidon2_params, poseidon3_params),
            &mut rng,
        )
        .unwrap();

        let mut nova = nova_params.initial_nova(z_0.clone()).unwrap();
        nova.prove_step(&mut rng, ext_input, None).unwrap();

        let ivc_proof = nova.ivc_proof();
        nova_params.verify(ivc_proof).unwrap();
    }

    #[test]
    #[ignore = "Nova IVC proof is too slow for CI in debug mode (~294s)"]
    fn test_innocence_circuit_multiple_steps() {
        let mut rng = StdRng::seed_from_u64(42);

        let tree = build_ofac_tree();
        let ofac_root = tree.root();
        let recipient = Fr::from(12345u64);

        // Find valid secrets for each transfer
        let secret1 = find_valid_secret(recipient, 1000);
        let secret2 = find_valid_secret(recipient, 2000);
        let secret3 = find_valid_secret(recipient, 3000);

        let addrs = [INNOCENT_ADDR_1, INNOCENT_ADDR_2, INNOCENT_ADDR_3];
        let secrets = [secret1, secret2, secret3];
        let values = [100u64, 200, 300];

        // Build transfer tree with all 3 transfers
        let mut transfer_tree = IncrementalMerkleTree::new(TRANSFER_DEPTH);
        let mut leaf_indices = vec![];
        for (i, addr) in addrs.iter().enumerate() {
            let from_address = ExclusionTree::parse_addresses(&[addr])[0];
            let burn_address =
                compute_burn_address_from_secret(recipient, secrets[i]).expect("valid PoW");
            let leaf_index = transfer_tree
                .insert(
                    fr_to_address(from_address),
                    fr_to_address(burn_address),
                    U256::from(values[i]),
                )
                .expect("insert should succeed");
            leaf_indices.push(leaf_index);
        }
        let transfer_root = transfer_tree.get_root();

        let z_0 = vec![ofac_root, recipient, transfer_root, Fr::ZERO, Fr::ZERO];

        let transfers: Vec<InnocenceExternalInputs<Fr, DEPTH, TRANSFER_DEPTH>> = addrs
            .iter()
            .zip(secrets.iter())
            .zip(values.iter())
            .enumerate()
            .map(|(i, ((addr, secret), value))| {
                let from_address = ExclusionTree::parse_addresses(&[addr])[0];
                let proof = tree
                    .prove_non_membership(from_address)
                    .expect("address should not be sanctioned");
                let transfer_proof = transfer_tree.prove(leaf_indices[i]);
                InnocenceExternalInputs::<Fr, DEPTH, TRANSFER_DEPTH> {
                    is_dummy: false,
                    from_address,
                    value: Fr::from(*value),
                    secret: *secret,
                    leaf_index: Fr::from(leaf_indices[i]),
                    transfer_siblings: transfer_proof.siblings.try_into().unwrap(),
                    start: proof.start,
                    end: proof.end,
                    gap_index: Fr::from(proof.gap_index),
                    siblings: proof.siblings_array(),
                }
            })
            .collect();

        let poseidon2_params = circom_poseidon2_config::<Fr>();
        let poseidon3_params = circom_poseidon3_config::<Fr>();
        let nova_params = NovaParams::<InnocenceCircuit<Fr, DEPTH, TRANSFER_DEPTH>>::rand(
            (poseidon2_params, poseidon3_params),
            &mut rng,
        )
        .unwrap();

        let mut nova = nova_params.initial_nova(z_0).unwrap();

        for ext_input in transfers {
            nova.prove_step(&mut rng, ext_input, None).unwrap();
        }

        // Add dummy steps to pad the batch (sequential indices starting after last real leaf)
        let last_real_index = *leaf_indices.last().unwrap();
        for i in 0..5 {
            let dummy = dummy_innocence_ext_input::<Fr, DEPTH, TRANSFER_DEPTH>(
                last_real_index + 1 + i as u64,
                Fr::ZERO,
            );
            nova.prove_step(&mut rng, dummy, None).unwrap();
        }

        let ivc_proof = nova.ivc_proof();
        nova_params.verify(ivc_proof).unwrap();

        // Verify final state: total = 100 + 200 + 300 = 600
        let final_state = nova.state();
        assert_eq!(final_state[0], ofac_root);
        assert_eq!(final_state[1], recipient);
        assert_eq!(final_state[2], transfer_root);
        assert_eq!(final_state[4], Fr::from(600u64));
    }

    #[test]
    #[ignore = "Nova IVC proof is too slow for CI in debug mode (~247s)"]
    fn test_innocence_circuit_with_dummy_padding() {
        let mut rng = StdRng::seed_from_u64(42);

        let tree = build_ofac_tree();
        let ofac_root = tree.root();
        let recipient = Fr::from(999u64);
        let secret = find_valid_secret(recipient, 5000);
        let burn_address = compute_burn_address_from_secret(recipient, secret).expect("valid PoW");

        let from_address = ExclusionTree::parse_addresses(&[INNOCENT_ADDR_1])[0];
        let proof = tree
            .prove_non_membership(from_address)
            .expect("address should not be sanctioned");

        let value = Fr::from(1000u64);

        // Build transfer tree
        let mut transfer_tree = IncrementalMerkleTree::new(TRANSFER_DEPTH);
        let leaf_index = transfer_tree
            .insert(
                fr_to_address(from_address),
                fr_to_address(burn_address),
                fr_to_u256(value),
            )
            .expect("insert should succeed");
        let transfer_root = transfer_tree.get_root();
        let transfer_proof = transfer_tree.prove(leaf_index);

        let z_0 = vec![ofac_root, recipient, transfer_root, Fr::ZERO, Fr::ZERO];

        let real_transfer = InnocenceExternalInputs::<Fr, DEPTH, TRANSFER_DEPTH> {
            is_dummy: false,
            from_address,
            value,
            secret,
            leaf_index: Fr::from(leaf_index),
            transfer_siblings: transfer_proof.siblings.try_into().unwrap(),
            start: proof.start,
            end: proof.end,
            gap_index: Fr::from(proof.gap_index),
            siblings: proof.siblings_array(),
        };

        let poseidon2_params = circom_poseidon2_config::<Fr>();
        let poseidon3_params = circom_poseidon3_config::<Fr>();
        let nova_params = NovaParams::<InnocenceCircuit<Fr, DEPTH, TRANSFER_DEPTH>>::rand(
            (poseidon2_params, poseidon3_params),
            &mut rng,
        )
        .unwrap();

        let mut nova = nova_params.initial_nova(z_0).unwrap();

        nova.prove_step(&mut rng, real_transfer, None).unwrap();

        // Dummy steps with zero value (sequential indices)
        for i in 0..3 {
            let dummy = dummy_innocence_ext_input::<Fr, DEPTH, TRANSFER_DEPTH>(
                leaf_index + 1 + i as u64,
                Fr::ZERO,
            );
            nova.prove_step(&mut rng, dummy, None).unwrap();
        }

        let ivc_proof = nova.ivc_proof();
        nova_params.verify(ivc_proof).unwrap();

        // Total should still be 1000
        let final_state = nova.state();
        assert_eq!(final_state[4], Fr::from(1000u64));
    }
}
