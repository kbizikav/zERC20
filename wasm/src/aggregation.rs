use crate::utils::{anyhow_to_js_error, fr_to_hex, parse_u256_hex};
use wasm_bindgen::prelude::*;
use zkp::{
    nova::constants::AGGREGATION_TREE_HEIGHT,
    utils::{convertion::u256_to_fr, tree::merkle_tree::MerkleTree},
};

#[wasm_bindgen]
pub fn aggregation_root(snapshot_hex: JsValue) -> Result<String, JsValue> {
    console_error_panic_hook::set_once();
    let snapshot: Vec<String> = serde_wasm_bindgen::from_value(snapshot_hex)
        .map_err(|err| JsValue::from_str(&err.to_string()))?;
    let root = compute_aggregation_root(&snapshot).map_err(anyhow_to_js_error)?;
    Ok(root)
}

#[wasm_bindgen]
pub fn aggregation_merkle_proof(snapshot_hex: JsValue, index: u32) -> Result<JsValue, JsValue> {
    console_error_panic_hook::set_once();
    let snapshot: Vec<String> = serde_wasm_bindgen::from_value(snapshot_hex)
        .map_err(|err| JsValue::from_str(&err.to_string()))?;
    let tree = build_aggregation_tree(&snapshot).map_err(anyhow_to_js_error)?;
    if index as usize >= (1usize << AGGREGATION_TREE_HEIGHT) {
        return Err(JsValue::from_str("aggregation index exceeds tree capacity"));
    }
    let proof = tree.prove(index as u64);
    let siblings: Vec<String> = proof.siblings.iter().map(fr_to_hex).collect();
    serde_wasm_bindgen::to_value(&siblings).map_err(|err| JsValue::from_str(&err.to_string()))
}

pub fn build_aggregation_tree(snapshot: &[String]) -> anyhow::Result<MerkleTree> {
    let mut tree = MerkleTree::new(AGGREGATION_TREE_HEIGHT);
    for (idx, hex_value) in snapshot.iter().enumerate() {
        let value = parse_u256_hex(hex_value)?;
        if value.is_zero() {
            continue;
        }
        tree.update_leaf(idx as u64, u256_to_fr(value));
    }
    Ok(tree)
}

fn compute_aggregation_root(snapshot: &[String]) -> anyhow::Result<String> {
    let tree = build_aggregation_tree(snapshot)?;
    Ok(fr_to_hex(&tree.get_root()))
}
