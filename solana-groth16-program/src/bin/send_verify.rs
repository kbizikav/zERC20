use anyhow::{anyhow, Result};
use clap::Parser;
use solana_client::rpc_client::RpcClient;
use solana_groth16_program::encode_precompile_instruction_data;
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
use std::time::{Duration, Instant};

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
    /// Path to precompile proof A bytes (64, BE)
    #[arg(long, env = "PROOF_A_PATH")]
    proof_a_path: String,
    /// Path to precompile proof B bytes (128, BE)
    #[arg(long, env = "PROOF_B_PATH")]
    proof_b_path: String,
    /// Path to precompile proof C bytes (64, BE)
    #[arg(long, env = "PROOF_C_PATH")]
    proof_c_path: String,
    /// Path to precompile public inputs bytes (N * 32, BE)
    #[arg(long, env = "PUBLIC_INPUTS_PATH")]
    public_inputs_path: String,
    /// Compute unit limit for the transaction
    #[arg(long, env = "CU_LIMIT", default_value_t = 1_000_000)]
    cu_limit: u32,
    /// Number of times to send the verification transaction
    #[arg(long, env = "ITERATIONS", default_value_t = 1)]
    iterations: u32,
    /// Sleep between iterations (milliseconds)
    #[arg(long, env = "SLEEP_MS", default_value_t = 0)]
    sleep_ms: u64,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let payer = read_keypair_file(&args.keypair).map_err(|e| anyhow!(e.to_string()))?;
    let program_id = args.program_id.parse::<Pubkey>()?;

    let proof_a = fs::read(&args.proof_a_path)?;
    let proof_b = fs::read(&args.proof_b_path)?;
    let proof_c = fs::read(&args.proof_c_path)?;
    let public_inputs = fs::read(&args.public_inputs_path)?;
    let proof_a: [u8; 64] = proof_a.try_into().map_err(|_| anyhow!("proof A must be 64 bytes"))?;
    let proof_b: [u8; 128] = proof_b.try_into().map_err(|_| anyhow!("proof B must be 128 bytes"))?;
    let proof_c: [u8; 64] = proof_c.try_into().map_err(|_| anyhow!("proof C must be 64 bytes"))?;
    let data = encode_precompile_instruction_data(&proof_a, &proof_b, &proof_c, &public_inputs)?;

    let budget_ix = ComputeBudgetInstruction::set_compute_unit_limit(args.cu_limit);
    let ix = Instruction {
        program_id,
        accounts: vec![],
        data,
    };

    let message = Message::new(&[budget_ix, ix], Some(&payer.pubkey()));
    let client = RpcClient::new_with_commitment(args.rpc_url.clone(), CommitmentConfig::processed());

    let mut cu_samples = Vec::new();
    let mut fee_samples = Vec::new();
    let mut latency_ms = Vec::new();
    let mut success_count = 0u32;

    for i in 0..args.iterations {
        let mut tx = Transaction::new_unsigned(message.clone());
        let blockhash = client.get_latest_blockhash()?;
        tx.sign(&[&payer], blockhash);

        let start = Instant::now();
        let sig = client.send_and_confirm_transaction(&tx)?;
        let elapsed = start.elapsed();
        latency_ms.push(elapsed.as_millis() as u64);
        success_count += 1;

        if args.iterations == 1 {
            println!("Submitted: {sig}");
        } else {
            println!("[{}/{}] {sig}", i + 1, args.iterations);
        }

        if let Ok(txn) = client.get_transaction(&sig, UiTransactionEncoding::Base64) {
            if let Some(meta) = txn.transaction.meta {
                let cu: Option<u64> = meta.compute_units_consumed.into();
                if let Some(cu) = cu {
                    cu_samples.push(cu);
                }
                fee_samples.push(meta.fee);
                if args.iterations == 1 {
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
        }

        if args.sleep_ms > 0 && i + 1 < args.iterations {
            std::thread::sleep(Duration::from_millis(args.sleep_ms));
        }
    }

    if args.iterations > 1 {
        println!(
            "Completed {success_count}/{} transactions",
            args.iterations
        );
        if !cu_samples.is_empty() {
            print_stats("Compute units", &mut cu_samples);
        }
        if !fee_samples.is_empty() {
            print_stats("Fee (lamports)", &mut fee_samples);
        }
        if !latency_ms.is_empty() {
            print_stats("Latency (ms)", &mut latency_ms);
        }
    }

    Ok(())
}

fn print_stats(label: &str, values: &mut Vec<u64>) {
    values.sort_unstable();
    let count = values.len() as u64;
    let sum: u64 = values.iter().sum();
    let avg = sum as f64 / count as f64;
    let p50 = percentile(values, 0.50);
    let p95 = percentile(values, 0.95);
    println!(
        "{label}: avg {:.2}, p50 {}, p95 {} (n={})",
        avg, p50, p95, count
    );
}

fn percentile(values: &[u64], p: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let idx = ((values.len() - 1) as f64 * p).round() as usize;
    values[idx.min(values.len() - 1)]
}
