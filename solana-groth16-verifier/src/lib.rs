#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::vec::Vec;
use ark_bn254::{Bn254, Fr};
use ark_groth16::{prepare_verifying_key, Groth16, Proof, VerifyingKey};
use ark_serialize::CanonicalDeserialize;
use ark_snark::SNARK;

const WITHDRAW_LOCAL_VK_BYTES: &[u8] =
    include_bytes!("../../nova_artifacts/withdraw_local_groth16_vk.bin");

/// Errors that can occur during verification.
#[derive(Debug)]
pub enum VerificationError {
    VerifyingKeyDeserialization,
    ProofDeserialization,
    PublicInputDeserialization,
    PublicInputLengthMismatch,
    ProofVerificationFailed,
    ProofInvalid,
}

impl core::fmt::Display for VerificationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let msg = match self {
            VerificationError::VerifyingKeyDeserialization => "failed to deserialize verifying key",
            VerificationError::ProofDeserialization => "failed to deserialize proof",
            VerificationError::PublicInputDeserialization => "failed to deserialize public input",
            VerificationError::PublicInputLengthMismatch => "public input length mismatch",
            VerificationError::ProofVerificationFailed => "verification failed internally",
            VerificationError::ProofInvalid => "invalid Groth16 proof",
        };
        f.write_str(msg)
    }
}

/// Verify a Groth16 proof for the withdraw-local circuit against the embedded verifying key.
///
/// - `proof_bytes`: compressed canonical arkworks serialization of `Proof<Bn254>`.
/// - `public_inputs_bytes`: concatenated compressed canonical `Fr` encodings.
pub fn verify_withdraw_local(
    proof_bytes: &[u8],
    public_inputs_bytes: &[u8],
) -> Result<(), VerificationError> {
    verify_with_vk_bytes(WITHDRAW_LOCAL_VK_BYTES, proof_bytes, public_inputs_bytes)
}

/// Verify a Groth16 proof given the verifying key bytes.
///
/// This keeps the public-input serialization identical to the generator:
/// compressed `Fr` values concatenated without a length prefix; the expected
/// count is derived from the verifying key (`gamma_abc_g1.len() - 1`).
pub fn verify_with_vk_bytes(
    vk_bytes: &[u8],
    proof_bytes: &[u8],
    public_inputs_bytes: &[u8],
) -> Result<(), VerificationError> {
    let mut vk_reader = vk_bytes;
    let vk = VerifyingKey::<Bn254>::deserialize_uncompressed(&mut vk_reader)
        .map_err(|_| VerificationError::VerifyingKeyDeserialization)?;
    let pvk = prepare_verifying_key(&vk);

    let mut proof_reader = proof_bytes;
    let proof = Proof::<Bn254>::deserialize_compressed(&mut proof_reader)
        .map_err(|_| VerificationError::ProofDeserialization)?;

    let expected_inputs = vk.gamma_abc_g1.len().saturating_sub(1);
    let mut reader = public_inputs_bytes;
    let mut public_inputs = Vec::with_capacity(expected_inputs);
    for _ in 0..expected_inputs {
        let fr = Fr::deserialize_compressed(&mut reader)
            .map_err(|_| VerificationError::PublicInputDeserialization)?;
        public_inputs.push(fr);
    }
    if !reader.is_empty() || public_inputs.len() != expected_inputs {
        return Err(VerificationError::PublicInputLengthMismatch);
    }

    let verified = Groth16::<Bn254>::verify_with_processed_vk(&pvk, &public_inputs, &proof)
        .map_err(|_| VerificationError::ProofVerificationFailed)?;
    if verified {
        Ok(())
    } else {
        Err(VerificationError::ProofInvalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn verify_withdraw_local_fixture() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let artifacts_dir = root.join("nova_artifacts");

        let proof_bytes = fs::read(artifacts_dir.join("withdraw_local_groth16_proof.bin")).unwrap();
        let public_inputs =
            fs::read(artifacts_dir.join("withdraw_local_groth16_public_inputs.bin")).unwrap();

        verify_withdraw_local(&proof_bytes, &public_inputs).expect("valid proof");
    }
}
