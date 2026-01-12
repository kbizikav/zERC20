use solana_groth16_program::{encode_precompile_instruction_data, process_instruction};
use solana_program::instruction::Instruction;
use solana_program::pubkey::Pubkey;
use solana_program_test::{processor, ProgramTest};
use solana_sdk::signer::Signer;
use solana_sdk::transaction::Transaction;

#[tokio::test]
async fn verify_fixture_proof_succeeds() {
    let program_id = Pubkey::new_unique();
    let pt = ProgramTest::new(
        "solana-groth16-program",
        program_id,
        processor!(process_instruction),
    );

    let (mut banks_client, payer, recent_blockhash) = pt.start().await;

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let artifacts = root.join("nova_artifacts");
    let proof_a =
        std::fs::read(artifacts.join("withdraw_local_groth16_precompile_proof_a.bin")).unwrap();
    let proof_b =
        std::fs::read(artifacts.join("withdraw_local_groth16_precompile_proof_b.bin")).unwrap();
    let proof_c =
        std::fs::read(artifacts.join("withdraw_local_groth16_precompile_proof_c.bin")).unwrap();
    let public_inputs =
        std::fs::read(artifacts.join("withdraw_local_groth16_precompile_public_inputs.bin"))
            .unwrap();

    let proof_a: [u8; 64] = proof_a.try_into().unwrap();
    let proof_b: [u8; 128] = proof_b.try_into().unwrap();
    let proof_c: [u8; 64] = proof_c.try_into().unwrap();
    let data =
        encode_precompile_instruction_data(&proof_a, &proof_b, &proof_c, &public_inputs).unwrap();
    let ix = Instruction {
        program_id,
        accounts: vec![],
        data,
    };

    let mut tx = Transaction::new_with_payer(&[ix], Some(&payer.pubkey()));
    tx.sign(&[&payer], recent_blockhash);

    banks_client.process_transaction(tx).await.unwrap();
}
