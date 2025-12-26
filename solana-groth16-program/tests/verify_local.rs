use solana_groth16_program::{encode_instruction_data, process_instruction};
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
    let proof = std::fs::read(artifacts.join("withdraw_local_groth16_proof.bin")).unwrap();
    let public_inputs =
        std::fs::read(artifacts.join("withdraw_local_groth16_public_inputs.bin")).unwrap();

    let data = encode_instruction_data(&proof, &public_inputs).unwrap();
    let ix = Instruction {
        program_id,
        accounts: vec![],
        data,
    };

    let mut tx = Transaction::new_with_payer(&[ix], Some(&payer.pubkey()));
    tx.sign(&[&payer], recent_blockhash);

    banks_client.process_transaction(tx).await.unwrap();
}
