use std::time::Instant;

use alloy::primitives::U256;
use ark_bn254::Fr;
use ark_ff::Zero;
use folding_schemes::FoldingScheme;
use rand::{SeedableRng, rngs::StdRng};
use zkp::nova::{
    constants::TRANSFER_TREE_HEIGHT,
    params::{DeciderParams, NovaParams},
    root_nova::{RootCircuit, RootExternalInputs},
    withdraw_nova::{WITHDRAW_STATE_LEN, WithdrawCircuit, dummy_withdraw_ext_input},
};
use zkp::utils::poseidon::utils::circom_poseidon_config;

fn main() {
    println!("== Nova decider proof benchmarks (single run) ==");
    println!("root_decider_secs = {:.3}", bench_root_decider());
    println!("withdraw_decider_secs = {:.3}", bench_withdraw_decider());
}

fn bench_root_decider() -> f64 {
    let poseidon_params = circom_poseidon_config::<Fr>();
    let mut setup_rng = StdRng::seed_from_u64(0xDEC1_DEAD);
    let nova_params = NovaParams::<RootCircuit<Fr>>::rand(poseidon_params.clone(), &mut setup_rng)
        .expect("root nova params");
    let state_len = nova_params.state_len().expect("root state length");

    let mut nova = nova_params
        .initial_nova(vec![Fr::zero(); state_len])
        .expect("root nova initialization");

    let mut step_rng = StdRng::seed_from_u64(0xF00D_FACE);
    let external_input = RootExternalInputs::<Fr> {
        is_dummy: true,
        address: Fr::zero(),
        value: Fr::zero(),
        siblings: [Fr::zero(); TRANSFER_TREE_HEIGHT],
    };
    nova.prove_step(&mut step_rng, external_input.clone(), None)
        .expect("root nova step proof");
    nova.prove_step(&mut step_rng, external_input, None)
        .expect("root nova step proof");

    let decider_params = DeciderParams::<RootCircuit<Fr>>::rand(&mut setup_rng, &nova_params)
        .expect("root decider params");

    let start = Instant::now();
    let _proof = decider_params
        .generate_decider_proof(nova)
        .expect("root decider proof");
    start.elapsed().as_secs_f64()
}

fn bench_withdraw_decider() -> f64 {
    let poseidon_params = circom_poseidon_config::<Fr>();
    let mut setup_rng = StdRng::seed_from_u64(0xA11C_EDA5);
    let nova_params = NovaParams::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>::rand(
        poseidon_params.clone(),
        &mut setup_rng,
    )
    .expect("withdraw nova params");

    let mut nova = nova_params
        .initial_nova(vec![Fr::zero(); WITHDRAW_STATE_LEN])
        .expect("withdraw nova initialization");

    let mut step_rng = StdRng::seed_from_u64(0xBEEF_CAFE);
    let external_input = dummy_withdraw_ext_input::<TRANSFER_TREE_HEIGHT>(1, U256::ZERO);
    nova.prove_step(&mut step_rng, external_input, None)
        .expect("withdraw nova step proof");
    let external_input = dummy_withdraw_ext_input::<TRANSFER_TREE_HEIGHT>(2, U256::ZERO);
    nova.prove_step(&mut step_rng, external_input, None)
        .expect("withdraw nova step proof");

    let decider_params = DeciderParams::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>::rand(
        &mut setup_rng,
        &nova_params,
    )
    .expect("withdraw decider params");

    let start = Instant::now();
    let _proof = decider_params
        .generate_decider_proof(nova)
        .expect("withdraw decider proof");
    start.elapsed().as_secs_f64()
}
