use anyhow::{anyhow, Result};
use clap::Parser;
use solana_client::rpc_client::RpcClient;
use solana_groth16_program::encode_instruction_data;
use solana_program::instruction::Instruction;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    compute_budget::ComputeBudgetInstruction,
    message::Message,
    pubkey::Pubkey,
    signature::{read_keypair_file, Signer},
    transaction::Transaction,
};
use solana_transaction_status::UiTransactionEncoding;
use std::fs;

#[derive(Parser, Debug)]
struct Args {
    /// RPC URL, e.g. http://127.0.0.1:8899
    #[arg(long, env = "RPC_URL")]
    rpc_url: String,
    /// Program ID of the deployed groth16 verifier program
    #[arg(long, env = "PROGRAM_ID")]
    program_id: String,
    /// Path to payer keypair (JSON)
    #[arg(long, env = "KEYPAIR")]
    keypair: String,
    /// Path to compressed Groth16 proof bytes
    #[arg(long, env = "PROOF_PATH")]
    proof_path: String,
    /// Path to compressed public inputs bytes
    #[arg(long, env = "PUBLIC_INPUTS_PATH")]
    public_inputs_path: String,
    /// Compute unit limit for the transaction
    #[arg(long, env = "CU_LIMIT", default_value_t = 1_000_000)]
    cu_limit: u32,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let payer = read_keypair_file(&args.keypair).map_err(|e| anyhow!(e.to_string()))?;
    let program_id = args.program_id.parse::<Pubkey>()?;

    let proof = fs::read(&args.proof_path)?;
    let public_inputs = fs::read(&args.public_inputs_path)?;
    let data = encode_instruction_data(&proof, &public_inputs)?;

    let budget_ix = ComputeBudgetInstruction::set_compute_unit_limit(args.cu_limit);
    let ix = Instruction {
        program_id,
        accounts: vec![],
        data,
    };

    let message = Message::new(&[budget_ix, ix], Some(&payer.pubkey()));
    let mut tx = Transaction::new_unsigned(message);

    let client = RpcClient::new_with_commitment(args.rpc_url.clone(), CommitmentConfig::processed());
    let blockhash = client.get_latest_blockhash()?;
    tx.sign(&[&payer], blockhash);

    let sig = client.send_and_confirm_transaction(&tx)?;
    println!("Submitted: {sig}");

    if let Ok(txn) = client.get_transaction(&sig, UiTransactionEncoding::Base64) {
        if let Some(meta) = txn.transaction.meta {
            let cu: Option<u64> = meta.compute_units_consumed.into();
            if let Some(cu) = cu {
                println!("Consumed compute units: {cu}");
            }
            let logs: Option<Vec<String>> = meta.log_messages.into();
            if let Some(logs) = logs {
                println!("Logs:");
                for log in logs {
                    println!("  {log}");
                }
            }
            println!("Fee (lamports): {}", meta.fee);
        }
    }

    Ok(())
}
