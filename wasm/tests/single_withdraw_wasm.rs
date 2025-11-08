#![cfg(target_arch = "wasm32")]

use alloy::primitives::{Address, U256};
use ark_bn254::Fr;
use serde::Deserialize;
use std::str::FromStr;
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;
use zkerc20_wasm::{JsSingleWithdrawInput, SingleWithdrawWasm, fr_to_hex};
use zkp::{
    circuits::burn_address::{compute_burn_address_from_secret, find_pow_nonce, secret_from_nonce},
    nova::constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
    utils::{
        convertion::{address_to_fr, fr_to_address, u256_to_fr},
        tree::incremental_merkle_tree::IncrementalMerkleTree,
    },
};

wasm_bindgen_test_configure!(run_in_browser);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Groth16ProveResult {
    proof_calldata: String,
    public_inputs: Vec<String>,
    tree_depth: usize,
}

#[wasm_bindgen_test]
fn test_single_withdraw_wasm_prove() {
    console_error_panic_hook::set_once();

    let recipient_address =
        Address::from_str("0x90f8bf6a479f320ead074411a4b0e7944ea8c9c1").unwrap();
    let recipient_fr = address_to_fr(recipient_address);
    let secret_seed = Fr::from(123_456u64);
    let nonce = find_pow_nonce(recipient_fr, secret_seed);
    let secret_fr = secret_from_nonce(secret_seed, nonce);
    let value_u256 = U256::from(5_000u64);
    let delta_fr = Fr::from(1_000u64);

    let local_witness = build_witness_input(
        TRANSFER_TREE_HEIGHT,
        recipient_fr,
        secret_fr,
        value_u256,
        delta_fr,
    );
    let global_witness = build_witness_input(
        GLOBAL_TRANSFER_TREE_HEIGHT,
        recipient_fr,
        secret_fr,
        value_u256,
        delta_fr,
    );

    let local_pk = include_bytes!("../../nova_artifacts/withdraw_local_groth16_pk.bin").to_vec();
    let local_vk = include_bytes!("../../nova_artifacts/withdraw_local_groth16_vk.bin").to_vec();
    let global_pk = include_bytes!("../../nova_artifacts/withdraw_global_groth16_pk.bin").to_vec();
    let global_vk = include_bytes!("../../nova_artifacts/withdraw_global_groth16_vk.bin").to_vec();

    let prover = SingleWithdrawWasm::new(local_pk, local_vk, global_pk, global_vk).unwrap();

    let local_js: JsValue = serde_wasm_bindgen::to_value(&local_witness).unwrap();
    let local_result_js = prover.prove(local_js).unwrap();
    let local_result: Groth16ProveResult = serde_wasm_bindgen::from_value(local_result_js).unwrap();

    assert!(local_result.proof_calldata.starts_with("0x"));
    assert!(local_result.proof_calldata.len() > 2);
    assert_eq!(local_result.tree_depth, TRANSFER_TREE_HEIGHT);
    assert_eq!(local_result.public_inputs.len(), 3);
    assert_eq!(local_result.public_inputs[0], local_witness.merkle_root);
    assert_eq!(local_result.public_inputs[1], local_witness.recipient);
    assert_eq!(local_result.public_inputs[2], local_witness.withdraw_value);

    let global_js: JsValue = serde_wasm_bindgen::to_value(&global_witness).unwrap();
    let global_result_js = prover.prove(global_js).unwrap();
    let global_result: Groth16ProveResult =
        serde_wasm_bindgen::from_value(global_result_js).unwrap();

    assert!(global_result.proof_calldata.starts_with("0x"));
    assert!(global_result.proof_calldata.len() > 2);
    assert_eq!(global_result.tree_depth, GLOBAL_TRANSFER_TREE_HEIGHT);
    assert_eq!(global_result.public_inputs.len(), 3);
    assert_eq!(global_result.public_inputs[0], global_witness.merkle_root);
    assert_eq!(global_result.public_inputs[1], global_witness.recipient);
    assert_eq!(
        global_result.public_inputs[2],
        global_witness.withdraw_value
    );
}

fn build_witness_input(
    tree_height: usize,
    recipient_fr: Fr,
    secret_fr: Fr,
    value_u256: U256,
    delta_fr: Fr,
) -> JsSingleWithdrawInput {
    let mut tree = IncrementalMerkleTree::new(tree_height);

    let leaf_address_fr =
        compute_burn_address_from_secret(recipient_fr, secret_fr).expect("secret satisfies PoW");
    let leaf_address = fr_to_address(leaf_address_fr);

    let index = tree.insert(leaf_address, value_u256);
    let proof = tree.prove(index);
    assert_eq!(proof.siblings.len(), tree_height);

    let root = tree.get_root();
    let value_fr = u256_to_fr(value_u256);
    let withdraw_value_fr = value_fr - delta_fr;

    JsSingleWithdrawInput {
        merkle_root: fr_to_hex(&root),
        recipient: fr_to_hex(&recipient_fr),
        withdraw_value: fr_to_hex(&withdraw_value_fr),
        value: fr_to_hex(&value_fr),
        delta: fr_to_hex(&delta_fr),
        secret: fr_to_hex(&secret_fr),
        leaf_index: index.to_string(),
        siblings: proof.siblings.iter().map(fr_to_hex).collect(),
    }
}
