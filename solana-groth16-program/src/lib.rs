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

mod precompile_verifier;
mod withdraw_local_vk;

use precompile_verifier::Groth16Verifier;
use withdraw_local_vk::WITHDRAW_LOCAL_VK;
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

    let (proof_a, proof_b, proof_c, public_inputs) = split_precompile_instruction_data(instruction_data)
        .map_err(|_| ProgramError::InvalidInstructionData)?;

    let mut verifier =
        Groth16Verifier::<3>::new(&proof_a, &proof_b, &proof_c, &public_inputs, &WITHDRAW_LOCAL_VK)
            .map_err(|err| {
                msg!("groth16 precompile verifier init failed: {}", err);
                ProgramError::Custom(0)
            })?;
    verifier.verify_unchecked().map_err(|err| {
        msg!("groth16 precompile verification failed: {}", err);
        ProgramError::Custom(0)
    })?;

    sol_log_compute_units();
    Ok(())
}

/// Build instruction data payload for `process_instruction` using precompile-friendly inputs.
/// Layout:
/// - proof_a (64 bytes)
/// - proof_b (128 bytes)
/// - proof_c (64 bytes)
/// - public inputs (N * 32 bytes, concatenated)
pub fn encode_precompile_instruction_data(
    proof_a: &[u8; 64],
    proof_b: &[u8; 128],
    proof_c: &[u8; 64],
    public_inputs: &[u8],
) -> Result<Vec<u8>, ProgramError> {
    if public_inputs.len() != 3 * 32 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mut buf = Vec::with_capacity(64 + 128 + 64 + public_inputs.len());
    buf.extend_from_slice(proof_a);
    buf.extend_from_slice(proof_b);
    buf.extend_from_slice(proof_c);
    buf.extend_from_slice(public_inputs);
    Ok(buf)
}

fn split_precompile_instruction_data(
    data: &[u8],
) -> Result<([u8; 64], [u8; 128], [u8; 64], [[u8; 32]; 3]), ()> {
    const PROOF_A_LEN: usize = 64;
    const PROOF_B_LEN: usize = 128;
    const PROOF_C_LEN: usize = 64;
    const PUB_INPUTS_LEN: usize = 3 * 32;
    const EXPECTED_LEN: usize = PROOF_A_LEN + PROOF_B_LEN + PROOF_C_LEN + PUB_INPUTS_LEN;

    if data.len() != EXPECTED_LEN {
        return Err(());
    }

    let proof_a: [u8; 64] = data[0..PROOF_A_LEN].try_into().map_err(|_| ())?;
    let proof_b: [u8; 128] = data[PROOF_A_LEN..PROOF_A_LEN + PROOF_B_LEN]
        .try_into()
        .map_err(|_| ())?;
    let proof_c: [u8; 64] = data[PROOF_A_LEN + PROOF_B_LEN..PROOF_A_LEN + PROOF_B_LEN + PROOF_C_LEN]
        .try_into()
        .map_err(|_| ())?;
    let public_inputs_slice = &data[PROOF_A_LEN + PROOF_B_LEN + PROOF_C_LEN..];
    let mut public_inputs = [[0u8; 32]; 3];
    for (i, chunk) in public_inputs_slice.chunks(32).enumerate() {
        public_inputs[i]
            .copy_from_slice(chunk);
    }
    Ok((proof_a, proof_b, proof_c, public_inputs))
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

        let proof_a = fs::read(
            artifacts_dir.join("withdraw_local_groth16_precompile_proof_a.bin"),
        )
        .unwrap();
        let proof_b = fs::read(
            artifacts_dir.join("withdraw_local_groth16_precompile_proof_b.bin"),
        )
        .unwrap();
        let proof_c = fs::read(
            artifacts_dir.join("withdraw_local_groth16_precompile_proof_c.bin"),
        )
        .unwrap();
        let public_inputs =
            fs::read(artifacts_dir.join("withdraw_local_groth16_precompile_public_inputs.bin"))
                .unwrap();

        let proof_a: [u8; 64] = proof_a.try_into().unwrap();
        let proof_b: [u8; 128] = proof_b.try_into().unwrap();
        let proof_c: [u8; 64] = proof_c.try_into().unwrap();
        let data =
            encode_precompile_instruction_data(&proof_a, &proof_b, &proof_c, &public_inputs)
                .unwrap();
        let (pa, pb, pc, inputs) = split_precompile_instruction_data(&data).unwrap();
        let mut verifier =
            Groth16Verifier::<3>::new(pa, pb, pc, inputs, &WITHDRAW_LOCAL_VK).unwrap();
        verifier.verify_unchecked().expect("proof verifies");
    }
}
