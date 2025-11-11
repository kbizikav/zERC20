#![cfg(target_arch = "wasm32")]

use alloy::primitives::{Address, U256};
use ark_bn254::Fr;
use ark_ff::AdditiveGroup;
use serde::Deserialize;
use std::str::FromStr;
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;
use web_time::Instant;
use zkerc20_wasm::{JsExternalInput, WithdrawNovaWasm, fr_to_hex, hex_to_fr, log_timing};
use zkp::{
    circuits::burn_address::{compute_burn_address_from_secret, find_pow_nonce, secret_from_nonce},
    nova::{
        constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
        params::NovaParams,
        withdraw_nova::{WITHDRAW_STATE_LEN, WithdrawCircuit, dummy_withdraw_ext_input},
    },
    utils::{
        convertion::{address_to_fr, fr_to_address, u256_to_fr},
        poseidon::utils::circom_poseidon_config,
        tree::incremental_merkle_tree::IncrementalMerkleTree,
    },
};

wasm_bindgen_test_configure!(run_in_browser);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProveResult {
    final_state: Vec<String>,
    ivc_proof: String,
    steps: usize,
}

#[wasm_bindgen_test]
fn test_withdraw_nova_wasm_prove() {
    console_error_panic_hook::set_once();

    let recipient =
        address_to_fr(Address::from_str("0x90f8bf6a479f320ead074411a4b0e7944ea8c9c1").unwrap());
    let secret_seeds = [
        Fr::from(123u64),
        Fr::from(456u64),
        Fr::from(789u64),
        Fr::from(101_112u64),
    ];
    let secrets_and_addresses = secret_seeds
        .iter()
        .map(|seed| {
            let nonce = find_pow_nonce(recipient, *seed);
            let secret = secret_from_nonce(*seed, nonce);
            let address = compute_burn_address_from_secret(recipient, secret)
                .expect("nonce should satisfy PoW");
            (secret, address)
        })
        .collect::<Vec<_>>();
    let secrets = secrets_and_addresses
        .iter()
        .map(|(secret, _)| *secret)
        .collect::<Vec<_>>();
    let addresses = secrets_and_addresses
        .iter()
        .map(|(_, address)| *address)
        .collect::<Vec<_>>();
    let values = vec![
        U256::from(1_000u64),
        U256::from(2_000u64),
        U256::from(3_000u64),
        U256::from(4_000u64),
    ];

    let mut tree = IncrementalMerkleTree::new(TRANSFER_TREE_HEIGHT);
    tree.insert(Address::ZERO, U256::ZERO);

    let mut indices = vec![];
    for i in 0..4 {
        let index = tree.insert(fr_to_address(addresses[i]), values[i]);
        indices.push(index);
    }
    let root = tree.get_root();

    let z0_state = vec![root, recipient, Fr::ZERO, Fr::ZERO];
    assert_eq!(z0_state.len(), WITHDRAW_STATE_LEN);

    let mut wasm_steps: Vec<JsExternalInput> = Vec::new();

    for (i, value) in values.iter().enumerate() {
        let leaf_index = indices[i];
        let proof = tree.prove(leaf_index);
        let siblings_vec = proof.siblings.clone();
        let sibling_hexes: Vec<String> = siblings_vec.iter().map(fr_to_hex).collect();
        let js_step = JsExternalInput {
            is_dummy: false,
            value: fr_to_hex(&u256_to_fr(*value)),
            secret: fr_to_hex(&secrets[i]),
            leaf_index: leaf_index.to_string(),
            siblings: sibling_hexes,
        };
        wasm_steps.push(js_step);
    }

    for i in 0..4 {
        let leaf_index = 5 + i as u64;
        let ext_input = dummy_withdraw_ext_input::<TRANSFER_TREE_HEIGHT>(leaf_index, U256::ZERO);

        let zero_sibling = fr_to_hex(&Fr::ZERO);
        let sibling_hexes = vec![zero_sibling.clone(); TRANSFER_TREE_HEIGHT];
        let js_step = JsExternalInput {
            is_dummy: true,
            value: fr_to_hex(&ext_input.value),
            secret: fr_to_hex(&ext_input.secret),
            leaf_index: leaf_index.to_string(),
            siblings: sibling_hexes,
        };
        wasm_steps.push(js_step);
    }

    let f_params = circom_poseidon_config::<Fr>();

    let local_nova_pp_bytes = include_bytes!("../../nova_artifacts/withdraw_local_nova_pp.bin");
    let local_nova_vp_bytes = include_bytes!("../../nova_artifacts/withdraw_local_nova_vp.bin");
    let global_nova_pp_bytes = include_bytes!("../../nova_artifacts/withdraw_global_nova_pp.bin");
    let global_nova_vp_bytes = include_bytes!("../../nova_artifacts/withdraw_global_nova_vp.bin");

    let local_pp_vec = local_nova_pp_bytes.to_vec();
    let local_vp_vec = local_nova_vp_bytes.to_vec();
    let global_pp_vec = global_nova_pp_bytes.to_vec();
    let global_vp_vec = global_nova_vp_bytes.to_vec();

    let _local_params = NovaParams::<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>::from_bytes(
        f_params.clone(),
        local_pp_vec.clone(),
        local_vp_vec.clone(),
    )
    .unwrap();

    // Ensure global artifacts deserialize correctly.
    let _global_params =
        NovaParams::<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>::from_bytes(
            f_params.clone(),
            global_pp_vec.clone(),
            global_vp_vec.clone(),
        )
        .unwrap();

    let load_start = Instant::now();
    let prover =
        WithdrawNovaWasm::new(local_pp_vec, local_vp_vec, global_pp_vec, global_vp_vec).unwrap();
    log_timing(&format!(
        "WithdrawNovaWasm::new {:.2} ms",
        load_start.elapsed().as_secs_f64() * 1_000.0
    ));
    let z0_hex: Vec<String> = z0_state.iter().map(fr_to_hex).collect();
    let z0_js: JsValue = serde_wasm_bindgen::to_value(&z0_hex).unwrap();
    let steps_js: JsValue = serde_wasm_bindgen::to_value(&wasm_steps).unwrap();

    let prove_start = Instant::now();
    let result_js = prover.prove(z0_js, steps_js).unwrap();
    log_timing(&format!(
        "WithdrawNovaWasm::prove (js bridge) {:.2} ms",
        prove_start.elapsed().as_secs_f64() * 1_000.0
    ));
    let result: ProveResult = serde_wasm_bindgen::from_value(result_js).unwrap();

    assert_eq!(result.steps, wasm_steps.len());
    assert!(result.ivc_proof.starts_with("0x"));
    let final_state: Vec<Fr> = result
        .final_state
        .iter()
        .map(|value| hex_to_fr(value).expect("final state element"))
        .collect();
    assert_eq!(final_state.len(), WITHDRAW_STATE_LEN);
    assert_eq!(final_state[0], root);
}
