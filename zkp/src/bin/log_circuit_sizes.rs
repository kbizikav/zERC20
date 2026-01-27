use anyhow::Result;
use ark_bn254::{Fr, G1Projective as G1};
use ark_crypto_primitives::sponge::poseidon::PoseidonConfig;
use ark_grumpkin::Projective as G2;
use folding_schemes::{
    arith::{Arith, r1cs::R1CS},
    commitment::{CommitmentScheme as _, pedersen::Pedersen},
    folding::{
        nova::{decider_eth::DeciderEthCircuit, get_r1cs, get_r1cs_from_cs},
        traits::Dummy,
    },
    frontend::FCircuit,
    transcript::poseidon::poseidon_canonical_config,
};
use rand::{SeedableRng, rngs::StdRng};

use zerc20_zkp::{
    groth16::withdraw::SingleWithdrawCircuit,
    nova::{
        constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
        params::FParams,
        root_nova::RootCircuit,
        withdraw_nova::WithdrawCircuit,
    },
    utils::poseidon::utils::{circom_poseidon2_config, circom_poseidon3_config},
};

fn main() -> Result<()> {
    let poseidon2_config = circom_poseidon2_config::<Fr>();
    let poseidon3_config = circom_poseidon3_config();

    log_circuit_sizes(&poseidon2_config, &poseidon3_config)?;

    Ok(())
}

fn log_circuit_sizes(
    poseidon2_config: &PoseidonConfig<Fr>,
    poseidon3_config: &PoseidonConfig<Fr>,
) -> Result<()> {
    println!("Circuit size report:");

    log_groth16_circuit::<TRANSFER_TREE_HEIGHT>(
        "groth16/withdraw_local",
        poseidon2_config,
        poseidon3_config,
    )?;
    log_groth16_circuit::<GLOBAL_TRANSFER_TREE_HEIGHT>(
        "groth16/withdraw_global",
        poseidon2_config,
        poseidon3_config,
    )?;

    let f_params = (poseidon2_config.clone(), poseidon3_config.clone());
    log_nova_circuit_sizes::<RootCircuit<Fr>>("root", f_params.clone())?;
    log_nova_circuit_sizes::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>(
        "withdraw_local",
        f_params.clone(),
    )?;
    log_nova_circuit_sizes::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>(
        "withdraw_global",
        f_params,
    )?;

    Ok(())
}

fn log_groth16_circuit<const DEPTH: usize>(
    label: &str,
    poseidon2_config: &PoseidonConfig<Fr>,
    poseidon3_config: &PoseidonConfig<Fr>,
) -> Result<()> {
    let zero = Fr::from(0u64);
    let circuit = SingleWithdrawCircuit::<Fr, DEPTH> {
        poseidon2_params: poseidon2_config.clone(),
        poseidon3_params: poseidon3_config.clone(),
        merkle_root: Some(zero),
        recipient: Some(zero),
        withdraw_value: Some(zero),
        from: Some(zero),
        value: Some(zero),
        delta: Some(zero),
        secret: Some(zero),
        leaf_index: Some(0),
        siblings: [(); DEPTH].map(|_| Some(zero)),
    };

    let r1cs = get_r1cs_from_cs(circuit)?;
    log_r1cs_stats(label, &r1cs);
    Ok(())
}

fn log_nova_circuit_sizes<C>(label: &str, f_params: FParams<C>) -> Result<()>
where
    C: FCircuit<Fr>,
    FParams<C>: Clone,
{
    let poseidon_config = poseidon_canonical_config::<Fr>();
    let circuit = C::new(f_params.clone())?;
    let state_len = circuit.state_len();
    let (r1cs, cf_r1cs) = get_r1cs::<G1, G2, C>(&poseidon_config, circuit)?;

    log_r1cs_stats(&format!("nova/{label}"), &r1cs);
    log_r1cs_stats(&format!("cyclefold/{label}"), &cf_r1cs);

    let kzg_len = r1cs.n_constraints().max(r1cs.n_witnesses());
    let pedersen_len = cf_r1cs.n_constraints().max(cf_r1cs.n_witnesses());
    println!(
        "commitment_len {label}: kzg_len={kzg_len} kzg_len_pow2={} pedersen_len={pedersen_len} pedersen_len_pow2={} state_len={state_len}",
        kzg_len.next_power_of_two(),
        pedersen_len.next_power_of_two()
    );

    let mut rng = StdRng::seed_from_u64(42);
    let (cf_pedersen_params, _) = Pedersen::<G2>::setup(&mut rng, pedersen_len)?;
    let decider_circuit = DeciderEthCircuit::<G1, G2>::dummy((
        r1cs.clone(),
        cf_r1cs.clone(),
        cf_pedersen_params,
        poseidon_config,
        (),
        (),
        state_len,
        2,
    ));
    let decider_r1cs = get_r1cs_from_cs(decider_circuit)?;
    log_r1cs_stats(&format!("decider/{label}"), &decider_r1cs);

    Ok(())
}

fn log_r1cs_stats<F: ark_ff::PrimeField>(label: &str, r1cs: &R1CS<F>) {
    let constraints = r1cs.n_constraints();
    let public_inputs = r1cs.n_public_inputs();
    let witness_vars = r1cs.n_witnesses();
    let instance_vars = public_inputs + 1;
    let total_constraints = constraints + instance_vars;
    let pot_degree = ark_std::log2(total_constraints.next_power_of_two());

    println!(
        "circuit_size {label}: constraints={constraints} public_inputs={public_inputs} witness_vars={witness_vars} total_with_instance={total_constraints} pot_degree=2^{pot_degree}"
    );
}
