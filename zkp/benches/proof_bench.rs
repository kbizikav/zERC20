use std::convert::TryInto;

use alloy::primitives::U256;
use ark_bn254::Fr;
use ark_crypto_primitives::sponge::poseidon::PoseidonConfig;
use ark_ff::Zero;
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use folding_schemes::FoldingScheme;
use rand::{SeedableRng, rngs::StdRng};

use zkp::circuits::burn_address::{
    compute_burn_address_from_secret, find_pow_nonce, secret_from_nonce,
};
use zkp::groth16::{params::Groth16Params, withdraw::SingleWithdrawCircuit};
use zkp::nova::{
    constants::TRANSFER_TREE_HEIGHT,
    params::NovaParams,
    root_nova::{RootCircuit, RootExternalInputs},
    withdraw_nova::{
        WITHDRAW_STATE_LEN, WithdrawCircuit, WithdrawExternalInputs, dummy_withdraw_ext_input,
    },
};
use zkp::utils::{
    poseidon::utils::circom_poseidon_config,
    tree::{gadgets::leaf_hash::compute_leaf_hash, merkle_tree::MerkleProof},
};

fn bench_root_nova_step(c: &mut Criterion) {
    let poseidon_params = circom_poseidon_config::<Fr>();
    let mut setup_rng = StdRng::seed_from_u64(0xDEADBEEF);
    let nova_params = NovaParams::<RootCircuit<Fr>>::rand(poseidon_params.clone(), &mut setup_rng)
        .expect("root nova params");
    let state_len = nova_params.state_len().expect("root state length");
    let base_nova = nova_params
        .initial_nova(vec![Fr::zero(); state_len])
        .expect("root nova initialization");
    let base_rng = StdRng::seed_from_u64(0xBAD5EED);
    let external_input = RootExternalInputs::<Fr> {
        is_dummy: true,
        address: Fr::zero(),
        value: Fr::zero(),
        siblings: [Fr::zero(); TRANSFER_TREE_HEIGHT],
    };

    c.bench_function("root_nova_step_dummy", |b| {
        b.iter_batched(
            || (base_nova.clone(), base_rng.clone()),
            |(mut nova, mut rng)| {
                nova.prove_step(&mut rng, external_input.clone(), None)
                    .expect("root step proof");
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_withdraw_nova_step(c: &mut Criterion) {
    let poseidon_params = circom_poseidon_config::<Fr>();
    let mut setup_rng = StdRng::seed_from_u64(0xA11CE5ED);
    let nova_params = NovaParams::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>::rand(
        poseidon_params.clone(),
        &mut setup_rng,
    )
    .expect("withdraw nova params");
    let initial_state = vec![Fr::zero(); WITHDRAW_STATE_LEN];
    let base_nova = nova_params
        .initial_nova(initial_state.clone())
        .expect("withdraw nova initialization");
    let base_rng = StdRng::seed_from_u64(0xFEE1DEAD);
    let external_input: WithdrawExternalInputs<Fr, TRANSFER_TREE_HEIGHT> =
        dummy_withdraw_ext_input(1, U256::ZERO);

    c.bench_function("withdraw_nova_step_dummy", |b| {
        b.iter_batched(
            || (base_nova.clone(), base_rng.clone()),
            |(mut nova, mut rng)| {
                nova.prove_step(&mut rng, external_input.clone(), None)
                    .expect("withdraw step proof");
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_single_withdraw_groth16(c: &mut Criterion) {
    const DEPTH: usize = TRANSFER_TREE_HEIGHT;
    let poseidon_config = circom_poseidon_config::<Fr>();
    let mut setup_rng = StdRng::seed_from_u64(0xC01DBEEF);
    let circuit_template = build_single_withdraw_circuit::<DEPTH>(&poseidon_config);
    let groth16_params =
        Groth16Params::rand(&mut setup_rng, circuit_template.clone()).expect("groth16 params");

    c.bench_function("single_withdraw_groth16_proof", |b| {
        b.iter_batched(
            || {
                (
                    build_single_withdraw_circuit::<DEPTH>(&poseidon_config),
                    StdRng::seed_from_u64(0xABCD1234),
                )
            },
            |(circuit, mut rng)| {
                let public_inputs = circuit.public_inputs().expect("public inputs");
                groth16_params
                    .generate_proof(&mut rng, circuit, &public_inputs)
                    .expect("groth16 proof");
            },
            BatchSize::SmallInput,
        );
    });
}

fn build_single_withdraw_circuit<const DEPTH: usize>(
    poseidon_config: &PoseidonConfig<Fr>,
) -> SingleWithdrawCircuit<Fr, DEPTH> {
    let recipient = Fr::from(321u64);
    let secret_seed = Fr::from(654u64);
    let nonce = find_pow_nonce(recipient, secret_seed);
    let secret = secret_from_nonce(secret_seed, nonce);
    let address =
        compute_burn_address_from_secret(recipient, secret).expect("nonce should satisfy PoW");
    let value = Fr::from(1_000u64);
    let delta = Fr::from(123u64);
    let leaf_index: u64 = 5;
    let withdraw_value = value - delta;

    let leaf = compute_leaf_hash(address, value);
    let proof = MerkleProof::dummy(DEPTH);
    let siblings: [Fr; DEPTH] = proof
        .siblings
        .clone()
        .try_into()
        .expect("dummy proof length matches depth");
    let merkle_root = proof.get_root(leaf, leaf_index);

    SingleWithdrawCircuit {
        poseidon_params: poseidon_config.clone(),
        merkle_root: Some(merkle_root),
        recipient: Some(recipient),
        withdraw_value: Some(withdraw_value),
        value: Some(value),
        delta: Some(delta),
        secret: Some(secret),
        leaf_index: Some(leaf_index),
        siblings: siblings.map(Some),
    }
}

criterion_group!(
    benches,
    bench_root_nova_step,
    bench_withdraw_nova_step,
    bench_single_withdraw_groth16
);
criterion_main!(benches);
