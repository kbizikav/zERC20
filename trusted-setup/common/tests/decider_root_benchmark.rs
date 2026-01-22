//! Benchmark test for DeciderRoot transcript creation.
//!
//! Run with: cargo test -p trusted-setup-common --test decider_root_benchmark -- --nocapture

use std::time::Instant;

use ark_bn254::{Bn254, Fr, G1Projective as G1};
use ark_grumpkin::Projective as G2;
use arkworks_phase2::{accumulator::Accumulator, transcript::Transcript};
use folding_schemes::{
    arith::Arith,
    commitment::{pedersen::Pedersen, CommitmentScheme},
    folding::{
        nova::{decider_eth::DeciderEthCircuit, get_r1cs},
        traits::Dummy,
    },
    frontend::FCircuit,
    transcript::poseidon::poseidon_canonical_config,
};
use rand::{rngs::StdRng, SeedableRng};
use std::cmp::max;
use zkp::{
    nova::root_nova::RootCircuit,
    utils::poseidon::utils::{circom_poseidon2_config, circom_poseidon3_config},
};

const PEDERSEN_SEED: u64 = 42;

fn build_decider_circuit_for_root() -> anyhow::Result<DeciderEthCircuit<G1, G2>> {
    let poseidon_config = poseidon_canonical_config::<Fr>();
    let poseidon2_config = circom_poseidon2_config::<Fr>();
    let poseidon3_config = circom_poseidon3_config();
    let circuit = RootCircuit::<Fr>::new((poseidon2_config, poseidon3_config))
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let state_len = circuit.state_len();
    let (r1cs, cf_r1cs) = get_r1cs::<G1, G2, RootCircuit<Fr>>(&poseidon_config, circuit)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let pedersen_len = max(cf_r1cs.n_constraints(), cf_r1cs.n_witnesses());
    let mut rng = StdRng::seed_from_u64(PEDERSEN_SEED);
    let (cf_cs_pp, _) = Pedersen::<G2>::setup(&mut rng, pedersen_len)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    Ok(DeciderEthCircuit::<G1, G2>::dummy((
        r1cs,
        cf_r1cs,
        cf_cs_pp,
        poseidon_config,
        (),
        (),
        state_len,
        2,
    )))
}

#[test]
#[ignore]
fn benchmark_decider_root_new_from_accumulator() {
    // Step 1: Load PTAU accumulator
    let ptau_path = trusted_setup_common::ptau_path_for_power(24);
    if !ptau_path.exists() {
        println!("PTAU file not found at {:?}", ptau_path);
        println!("Please download it first. Skipping test.");
        return;
    }

    println!("\n=== DeciderRoot new_from_accumulator Benchmark ===\n");

    let start = Instant::now();
    let accum = Accumulator::<Bn254>::from_ptau_file(&ptau_path).expect("failed to load ptau");
    let load_time = start.elapsed();
    println!("[1] PTAU load time: {:?}", load_time);

    // Step 2: Build decider circuit
    let start = Instant::now();
    let circuit = build_decider_circuit_for_root().expect("failed to build decider circuit");
    let circuit_time = start.elapsed();
    println!("[2] Decider circuit build time: {:?}", circuit_time);

    // Step 3: Create transcript from accumulator
    let start = Instant::now();
    let _transcript = Transcript::new_from_accumulator(&accum, circuit)
        .expect("failed to create transcript from accumulator");
    let transcript_time = start.elapsed();
    println!(
        "[3] Transcript::new_from_accumulator time: {:?}",
        transcript_time
    );

    println!("\n=== Summary ===");
    println!(
        "Total time: {:?}",
        load_time + circuit_time + transcript_time
    );
    println!(
        "  [1] PTAU load:              {:?} ({:.1}%)",
        load_time,
        100.0 * load_time.as_secs_f64()
            / (load_time + circuit_time + transcript_time).as_secs_f64()
    );
    println!(
        "  [2] Circuit build:          {:?} ({:.1}%)",
        circuit_time,
        100.0 * circuit_time.as_secs_f64()
            / (load_time + circuit_time + transcript_time).as_secs_f64()
    );
    println!(
        "  [3] new_from_accumulator:   {:?} ({:.1}%)",
        transcript_time,
        100.0 * transcript_time.as_secs_f64()
            / (load_time + circuit_time + transcript_time).as_secs_f64()
    );
}

#[test]
#[ignore]
fn benchmark_circuit_build_detailed() {
    println!("\n=== DeciderRoot Circuit Build Detailed Benchmark ===\n");

    // Step 2a: Poseidon config
    let start = Instant::now();
    let poseidon_config = poseidon_canonical_config::<Fr>();
    let poseidon2_config = circom_poseidon2_config::<Fr>();
    let poseidon3_config = circom_poseidon3_config();
    let poseidon_time = start.elapsed();
    println!("[2a] Poseidon config time: {:?}", poseidon_time);

    // Step 2b: RootCircuit::new
    let start = Instant::now();
    let circuit = RootCircuit::<Fr>::new((poseidon2_config.clone(), poseidon3_config.clone()))
        .expect("failed to create circuit");
    let circuit_new_time = start.elapsed();
    println!("[2b] RootCircuit::new time: {:?}", circuit_new_time);

    let state_len = circuit.state_len();
    println!("     state_len: {}", state_len);

    // Step 2c: get_r1cs
    let start = Instant::now();
    let (r1cs, cf_r1cs) =
        get_r1cs::<G1, G2, RootCircuit<Fr>>(&poseidon_config, circuit).expect("failed to get r1cs");
    let r1cs_time = start.elapsed();
    println!("[2c] get_r1cs time: {:?}", r1cs_time);
    println!(
        "     r1cs constraints: {}, witnesses: {}",
        r1cs.n_constraints(),
        r1cs.n_witnesses()
    );
    println!(
        "     cf_r1cs constraints: {}, witnesses: {}",
        cf_r1cs.n_constraints(),
        cf_r1cs.n_witnesses()
    );

    // Step 2d: Pedersen setup
    let pedersen_len = max(cf_r1cs.n_constraints(), cf_r1cs.n_witnesses());
    println!("     pedersen_len: {}", pedersen_len);

    let start = Instant::now();
    let mut rng = StdRng::seed_from_u64(PEDERSEN_SEED);
    let (cf_cs_pp, _) = Pedersen::<G2>::setup(&mut rng, pedersen_len).expect("pedersen setup");
    let pedersen_time = start.elapsed();
    println!("[2d] Pedersen::setup time: {:?}", pedersen_time);

    // Step 2e: DeciderEthCircuit::dummy
    let start = Instant::now();
    let poseidon_config2 = poseidon_canonical_config::<Fr>();
    let _decider_circuit = DeciderEthCircuit::<G1, G2>::dummy((
        r1cs,
        cf_r1cs,
        cf_cs_pp,
        poseidon_config2,
        (),
        (),
        state_len,
        2,
    ));
    let dummy_time = start.elapsed();
    println!("[2e] DeciderEthCircuit::dummy time: {:?}", dummy_time);

    println!("\n=== Circuit Build Summary ===");
    let total = poseidon_time + circuit_new_time + r1cs_time + pedersen_time + dummy_time;
    println!("Total circuit build time: {:?}", total);
}
