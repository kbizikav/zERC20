use crate::{
    circuits::withdraw::single_withdraw, utils::poseidon::gadgets::CircomCRHParametersVar,
};
use ark_crypto_primitives::sponge::{Absorb, poseidon::PoseidonConfig};
use ark_ff::PrimeField;
use ark_r1cs_std::{alloc::AllocVar, eq::EqGadget, fields::fp::FpVar};
use ark_relations::gr1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use ark_relations::ns;

#[derive(Clone)]
pub struct SingleWithdrawCircuit<F: PrimeField + Absorb, const DEPTH: usize> {
    pub poseidon_params: PoseidonConfig<F>,

    // ---- public inputs ----
    pub merkle_root: Option<F>,
    pub recipient: Option<F>,
    pub withdraw_value: Option<F>,

    // ---- witness ----
    pub value: Option<F>,
    pub delta: Option<F>,
    pub secret: Option<F>,
    pub leaf_index: Option<u64>,
    pub siblings: [Option<F>; DEPTH],
}

impl<F: PrimeField + Absorb, const DEPTH: usize> SingleWithdrawCircuit<F, DEPTH> {
    pub fn new(poseidon_params: PoseidonConfig<F>) -> Self {
        Self {
            poseidon_params,
            merkle_root: None,
            recipient: None,
            withdraw_value: None,
            value: None,
            delta: None,
            secret: None,
            leaf_index: None,
            siblings: [(); DEPTH].map(|_| None),
        }
    }

    pub fn public_inputs(&self) -> Result<Vec<F>, SynthesisError> {
        Ok(vec![
            self.merkle_root.ok_or(SynthesisError::AssignmentMissing)?,
            self.recipient.ok_or(SynthesisError::AssignmentMissing)?,
            self.withdraw_value
                .ok_or(SynthesisError::AssignmentMissing)?,
        ])
    }
}

impl<F: PrimeField + Absorb, const DEPTH: usize> ConstraintSynthesizer<F>
    for SingleWithdrawCircuit<F, DEPTH>
{
    fn generate_constraints(self, cs: ConstraintSystemRef<F>) -> Result<(), SynthesisError> {
        let Self {
            poseidon_params,
            merkle_root,
            recipient,
            withdraw_value,
            value,
            delta,
            secret,
            leaf_index,
            siblings,
        } = self;

        let poseidon_params =
            CircomCRHParametersVar::new_constant(ns!(cs, "poseidon_params"), &poseidon_params)?;

        // ---- public inputs ----
        let merkle_root = FpVar::<F>::new_input(ns!(cs, "merkle_root"), || {
            merkle_root.ok_or(SynthesisError::AssignmentMissing)
        })?;
        let recipient = FpVar::<F>::new_input(ns!(cs, "recipient"), || {
            recipient.ok_or(SynthesisError::AssignmentMissing)
        })?;
        let expected_withdraw_value = FpVar::<F>::new_input(ns!(cs, "withdraw_value"), || {
            withdraw_value.ok_or(SynthesisError::AssignmentMissing)
        })?;

        // ---- witness ----
        let value = FpVar::<F>::new_witness(ns!(cs, "value"), || {
            value.ok_or(SynthesisError::AssignmentMissing)
        })?;
        let delta = FpVar::<F>::new_witness(ns!(cs, "delta"), || {
            delta.ok_or(SynthesisError::AssignmentMissing)
        })?;
        let secret = FpVar::<F>::new_witness(ns!(cs, "secret"), || {
            secret.ok_or(SynthesisError::AssignmentMissing)
        })?;
        let leaf_index = FpVar::<F>::new_witness(ns!(cs, "leaf_index"), || {
            leaf_index
                .map(F::from)
                .ok_or(SynthesisError::AssignmentMissing)
        })?;
        let sibling_vars = siblings
            .into_iter()
            .map(|sibling| {
                FpVar::<F>::new_witness(ns!(cs, "sibling"), || {
                    sibling.ok_or(SynthesisError::AssignmentMissing)
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let computed_withdraw_value = single_withdraw::<F, DEPTH>(
            &poseidon_params,
            &merkle_root,
            &recipient,
            &value,
            &delta,
            &secret,
            &leaf_index,
            sibling_vars.as_slice(),
        )?;

        expected_withdraw_value.enforce_equal(&computed_withdraw_value)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SingleWithdrawCircuit;
    use crate::circuits::burn_address::{
        compute_burn_address_from_secret, find_pow_nonce, secret_from_nonce,
    };
    use crate::groth16::params::Groth16Params;
    use crate::test_utils::merkle_root_from_path;
    use crate::utils::poseidon::utils::circom_poseidon_config;
    use crate::utils::tree::gadgets::leaf_hash::compute_leaf_hash;
    use alloy::primitives::keccak256;
    use ark_bn254::Fr;
    use ark_std::rand::{SeedableRng, rngs::StdRng};
    use solidity_verifiers::evm::{Evm, compile_solidity};

    const DEPTH: usize = 4;

    #[test]
    fn test_single_withdraw_circuit() {
        let poseidon_config = circom_poseidon_config();

        let recipient_value = Fr::from(321u64);
        let secret_seed = Fr::from(654u64);
        let nonce = find_pow_nonce(recipient_value, secret_seed);
        let secret_value = secret_from_nonce(secret_seed, nonce);
        let address_value = compute_burn_address_from_secret(recipient_value, secret_value)
            .expect("nonce should satisfy PoW");
        let value_value = Fr::from(100u64);
        let delta_value = Fr::from(25u64);
        let withdraw_value_value = value_value - delta_value;
        let leaf_index_value: u64 = 3;
        let siblings_values: [Fr; DEPTH] = [
            Fr::from(5u64),
            Fr::from(6u64),
            Fr::from(7u64),
            Fr::from(8u64),
        ];

        let leaf_value = compute_leaf_hash(address_value, value_value);
        let merkle_root_value = merkle_root_from_path(
            &poseidon_config,
            leaf_index_value,
            leaf_value,
            &siblings_values,
        );

        let circuit = SingleWithdrawCircuit::<Fr, DEPTH> {
            poseidon_params: poseidon_config.clone(),
            merkle_root: Some(merkle_root_value),
            recipient: Some(recipient_value),
            withdraw_value: Some(withdraw_value_value),
            value: Some(value_value),
            delta: Some(delta_value),
            secret: Some(secret_value),
            leaf_index: Some(leaf_index_value),
            siblings: siblings_values.map(Some),
        };

        let mut rng = StdRng::seed_from_u64(42);
        let params = Groth16Params::rand(&mut rng, circuit.clone()).expect("setup");

        let solidity_source = params.verifier_solidity_code().expect("valid solidity");
        let bytecode = compile_solidity(solidity_source.as_bytes(), "Groth16Verifier");

        let mut evm = Evm::default();
        let verifier_address = evm.create(bytecode);

        let public_inputs = circuit.public_inputs().expect("public inputs");
        let proof = params
            .generate_proof(&mut rng, circuit, &public_inputs)
            .expect("groth16 proof calldata");

        let selector_input = format!(
            "verifyProof(uint256[2],uint256[2][2],uint256[2],uint256[{}])",
            params.vk.gamma_abc_g1.len() - 1
        );
        let selector = keccak256(selector_input.as_bytes());
        let mut calldata = selector[..4].to_vec();
        calldata.extend_from_slice(&proof);

        let (_, output) = evm.call(verifier_address, calldata.clone());
        assert_eq!(output.last(), Some(&1u8));

        let mut invalid_calldata = calldata.clone();
        let last = invalid_calldata.last_mut().expect("calldata is non-empty");
        *last ^= 1;
        let (_, output) = evm.call(verifier_address, invalid_calldata);
        assert_eq!(output.last(), Some(&0u8));
    }
}
