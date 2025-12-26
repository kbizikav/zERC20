//! Minimal Solana program that verifies a withdraw-local Groth16 proof and logs compute units.
//! Instruction data layout:
//! - `u16` proof_len (LE)
//! - `u16` public_inputs_len (LE)
//! - `proof_len` bytes of compressed `Proof<Bn254>`
//! - `public_inputs_len` bytes of concatenated compressed `Fr` public inputs

#![cfg_attr(not(test), no_std)]
#![allow(unexpected_cfgs)]

extern crate alloc;
use alloc::{format, vec::Vec};

use solana_groth16_verifier::verify_withdraw_local;
use solana_program::{
    account_info::AccountInfo,
    entrypoint,
    entrypoint::ProgramResult,
    log::sol_log_compute_units,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
};

entrypoint!(process_instruction);

pub fn process_instruction(
    _program_id: &Pubkey,
    _accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    sol_log_compute_units();

    let (proof_bytes, public_inputs) = split_instruction_data(instruction_data)
        .map_err(|_| ProgramError::InvalidInstructionData)?;

    verify_withdraw_local(proof_bytes, public_inputs).map_err(|err| {
        msg!("groth16 verification failed: {}", err);
        ProgramError::Custom(0)
    })?;

    sol_log_compute_units();
    Ok(())
}

/// Build instruction data payload for `process_instruction`.
/// Layout:
/// - u16 proof length (LE)
/// - u16 public inputs length (LE)
/// - proof bytes
/// - public input bytes
pub fn encode_instruction_data(proof: &[u8], public_inputs: &[u8]) -> Result<Vec<u8>, ProgramError> {
    if proof.len() > u16::MAX as usize || public_inputs.len() > u16::MAX as usize {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mut buf = Vec::with_capacity(4 + proof.len() + public_inputs.len());
    buf.extend_from_slice(&(proof.len() as u16).to_le_bytes());
    buf.extend_from_slice(&(public_inputs.len() as u16).to_le_bytes());
    buf.extend_from_slice(proof);
    buf.extend_from_slice(public_inputs);
    Ok(buf)
}

fn split_instruction_data(data: &[u8]) -> Result<(&[u8], &[u8]), ()> {
    if data.len() < 4 {
        return Err(());
    }

    let proof_len = u16::from_le_bytes([data[0], data[1]]) as usize;
    let public_inputs_len = u16::from_le_bytes([data[2], data[3]]) as usize;
    let expected_len = 4 + proof_len + public_inputs_len;
    if data.len() != expected_len {
        return Err(());
    }

    let proof_bytes = &data[4..4 + proof_len];
    let public_inputs = &data[4 + proof_len..expected_len];
    Ok((proof_bytes, public_inputs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn splits_and_verifies_fixture() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let artifacts_dir = root.join("nova_artifacts");

        let proof = fs::read(artifacts_dir.join("withdraw_local_groth16_proof.bin")).unwrap();
        let public_inputs =
            fs::read(artifacts_dir.join("withdraw_local_groth16_public_inputs.bin")).unwrap();

        let data = encode_instruction_data(&proof, &public_inputs).unwrap();
        let (proof_bytes, inputs) = split_instruction_data(&data).unwrap();
        verify_withdraw_local(proof_bytes, inputs).expect("proof verifies");
    }
}
