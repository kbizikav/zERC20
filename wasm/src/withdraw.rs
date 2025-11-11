use crate::utils::{anyhow_to_js_error, fr_to_hex, hex_to_fr, log_timing, parse_u64};
use ark_bn254::Fr;
use ark_serialize::CanonicalSerialize;
use folding_schemes::FoldingScheme;
use rand::{SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use web_time::Instant;
use zkp::{
    groth16::{params::Groth16Params, withdraw::SingleWithdrawCircuit},
    nova::{
        constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
        params::NovaParams,
        withdraw_nova::{WITHDRAW_STATE_LEN, WithdrawCircuit, WithdrawExternalInputs},
    },
    utils::poseidon::utils::circom_poseidon_config,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsExternalInput {
    pub is_dummy: bool,
    pub value: String,
    pub secret: String,
    pub leaf_index: String,
    pub siblings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsProveResult {
    final_state: Vec<String>,
    ivc_proof: String,
    steps: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsSingleWithdrawInput {
    pub merkle_root: String,
    pub recipient: String,
    pub withdraw_value: String,
    pub value: String,
    pub delta: String,
    pub secret: String,
    pub leaf_index: String,
    pub siblings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsGroth16ProveResult {
    proof_calldata: String,
    public_inputs: Vec<String>,
    tree_depth: usize,
}

#[wasm_bindgen]
pub struct WithdrawNovaWasm {
    local: NovaParams<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>,
    global: NovaParams<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>,
}

#[wasm_bindgen]
impl WithdrawNovaWasm {
    #[wasm_bindgen(constructor)]
    pub fn new(
        local_pp_bytes: Vec<u8>,
        local_vp_bytes: Vec<u8>,
        global_pp_bytes: Vec<u8>,
        global_vp_bytes: Vec<u8>,
    ) -> Result<WithdrawNovaWasm, JsValue> {
        console_error_panic_hook::set_once();
        let f_params = circom_poseidon_config();
        let local = NovaParams::from_bytes(f_params.clone(), local_pp_bytes, local_vp_bytes)
            .map_err(|err| JsValue::from_str(&err.to_string()))?;
        let global = NovaParams::from_bytes(f_params, global_pp_bytes, global_vp_bytes)
            .map_err(|err| JsValue::from_str(&err.to_string()))?;
        Ok(WithdrawNovaWasm { local, global })
    }

    #[wasm_bindgen]
    pub fn prove(&self, z0: JsValue, steps: JsValue) -> Result<JsValue, JsValue> {
        console_error_panic_hook::set_once();

        let z0_hex: Vec<String> = serde_wasm_bindgen::from_value(z0)
            .map_err(|err| JsValue::from_str(&err.to_string()))?;
        let step_inputs: Vec<JsExternalInput> = serde_wasm_bindgen::from_value(steps)
            .map_err(|err| JsValue::from_str(&err.to_string()))?;
        if z0_hex.len() != WITHDRAW_STATE_LEN {
            return Err(JsValue::from_str(
                "z0 must contain exactly four field elements",
            ));
        }
        let z0_fields = z0_hex
            .iter()
            .map(|value| hex_to_fr(value))
            .collect::<Result<Vec<_>, _>>()
            .map_err(anyhow_to_js_error)?;
        let tree_height = step_inputs
            .first()
            .map(|input| input.siblings.len())
            .unwrap_or(TRANSFER_TREE_HEIGHT);

        if step_inputs
            .iter()
            .any(|input| input.siblings.len() != tree_height)
        {
            return Err(JsValue::from_str(
                "all steps must have sibling paths with uniform length",
            ));
        }

        match tree_height {
            TRANSFER_TREE_HEIGHT => {
                prove_with_depth::<TRANSFER_TREE_HEIGHT>(&self.local, z0_fields, &step_inputs)
            }
            GLOBAL_TRANSFER_TREE_HEIGHT => prove_with_depth::<GLOBAL_TRANSFER_TREE_HEIGHT>(
                &self.global,
                z0_fields,
                &step_inputs,
            ),
            _ => Err(JsValue::from_str(&format!(
                "unsupported sibling path length: {tree_height} (expected {TRANSFER_TREE_HEIGHT} or {GLOBAL_TRANSFER_TREE_HEIGHT})"
            ))),
        }
    }
}

#[wasm_bindgen]
pub struct SingleWithdrawWasm {
    local: Groth16Params,
    global: Groth16Params,
}

#[wasm_bindgen]
impl SingleWithdrawWasm {
    #[wasm_bindgen(constructor)]
    pub fn new(
        local_pk_bytes: Vec<u8>,
        local_vk_bytes: Vec<u8>,
        global_pk_bytes: Vec<u8>,
        global_vk_bytes: Vec<u8>,
    ) -> Result<SingleWithdrawWasm, JsValue> {
        console_error_panic_hook::set_once();
        let local = Groth16Params::from_bytes(local_pk_bytes, local_vk_bytes)
            .map_err(|err| JsValue::from_str(&err.to_string()))?;
        let global = Groth16Params::from_bytes(global_pk_bytes, global_vk_bytes)
            .map_err(|err| JsValue::from_str(&err.to_string()))?;

        Ok(SingleWithdrawWasm { local, global })
    }

    #[wasm_bindgen]
    pub fn prove(&self, witness: JsValue) -> Result<JsValue, JsValue> {
        console_error_panic_hook::set_once();
        let witness: JsSingleWithdrawInput = serde_wasm_bindgen::from_value(witness)
            .map_err(|err| JsValue::from_str(&err.to_string()))?;

        let tree_depth = witness.siblings.len();
        match tree_depth {
            TRANSFER_TREE_HEIGHT => {
                prove_single_with_depth::<TRANSFER_TREE_HEIGHT>(&self.local, witness)
            }
            GLOBAL_TRANSFER_TREE_HEIGHT => {
                prove_single_with_depth::<GLOBAL_TRANSFER_TREE_HEIGHT>(&self.global, witness)
            }
            _ => Err(JsValue::from_str(&format!(
                "unsupported sibling path length: {tree_depth} (expected {TRANSFER_TREE_HEIGHT} or {GLOBAL_TRANSFER_TREE_HEIGHT})"
            ))),
        }
    }
}

fn prove_with_depth<const DEPTH: usize>(
    params: &NovaParams<WithdrawCircuit<Fr, DEPTH>>,
    z0_fields: Vec<Fr>,
    step_inputs: &[JsExternalInput],
) -> Result<JsValue, JsValue> {
    let prove_start = Instant::now();
    let mut nova = params
        .initial_nova(z0_fields)
        .map_err(|err| JsValue::from_str(&err.to_string()))?;

    let mut rng = rand::thread_rng();

    for (idx, external) in step_inputs.iter().enumerate() {
        let step_start = Instant::now();
        let siblings_vec = external
            .siblings
            .iter()
            .map(|sibling| hex_to_fr(sibling))
            .collect::<Result<Vec<_>, _>>()
            .map_err(anyhow_to_js_error)?;

        if siblings_vec.len() != DEPTH {
            return Err(JsValue::from_str(&format!(
                "Merkle path must have {DEPTH} siblings, found {}",
                siblings_vec.len()
            )));
        }

        let siblings: [Fr; DEPTH] = siblings_vec
            .try_into()
            .map_err(|_| JsValue::from_str("failed to convert siblings into fixed-size array"))?;

        let leaf_index = parse_u64(&external.leaf_index).map_err(anyhow_to_js_error)?;
        let value_fr = hex_to_fr(&external.value).map_err(anyhow_to_js_error)?;
        let secret_fr = hex_to_fr(&external.secret).map_err(anyhow_to_js_error)?;

        let ext_input = WithdrawExternalInputs {
            is_dummy: if external.is_dummy {
                Fr::from(1u64)
            } else {
                Fr::from(0u64)
            },
            value: value_fr,
            secret: secret_fr,
            leaf_index: Fr::from(leaf_index),
            siblings,
        };

        nova.prove_step(&mut rng, ext_input, None)
            .map_err(|err| JsValue::from_str(&format!("prove_step failed at {idx}: {err}")))?;
        log_timing(&format!(
            "WithdrawNovaWasm::prove_step[{idx}] {duration:.2} ms",
            duration = step_start.elapsed().as_secs_f64() * 1_000.0
        ));
    }

    log_timing(&format!(
        "WithdrawNovaWasm::prove total {:.2} ms for {} steps",
        prove_start.elapsed().as_secs_f64() * 1_000.0,
        step_inputs.len()
    ));

    let state = nova.state();
    let final_state = state.iter().map(fr_to_hex).collect::<Vec<_>>();
    let ivc_proof = nova.ivc_proof();
    params
        .verify(ivc_proof.clone())
        .map_err(|err| JsValue::from_str(&err.to_string()))?;
    let mut proof_bytes = Vec::new();
    ivc_proof
        .serialize_uncompressed(&mut proof_bytes)
        .map_err(|err| JsValue::from_str(&err.to_string()))?;
    let result = JsProveResult {
        final_state,
        ivc_proof: format!("0x{}", hex::encode(proof_bytes)),
        steps: step_inputs.len(),
    };
    serde_wasm_bindgen::to_value(&result).map_err(|err| JsValue::from_str(&err.to_string()))
}

fn prove_single_with_depth<const DEPTH: usize>(
    params: &Groth16Params,
    witness: JsSingleWithdrawInput,
) -> Result<JsValue, JsValue> {
    let prove_start = Instant::now();
    let JsSingleWithdrawInput {
        merkle_root,
        recipient,
        withdraw_value,
        value,
        delta,
        secret,
        leaf_index,
        siblings,
    } = witness;

    if siblings.len() != DEPTH {
        return Err(JsValue::from_str(&format!(
            "Merkle path must have {DEPTH} siblings, found {}",
            siblings.len()
        )));
    }

    let poseidon_params = circom_poseidon_config();

    let merkle_root_fr = hex_to_fr(&merkle_root).map_err(anyhow_to_js_error)?;
    let recipient_fr = hex_to_fr(&recipient).map_err(anyhow_to_js_error)?;
    let withdraw_value_fr = hex_to_fr(&withdraw_value).map_err(anyhow_to_js_error)?;
    let value_fr = hex_to_fr(&value).map_err(anyhow_to_js_error)?;
    let delta_fr = hex_to_fr(&delta).map_err(anyhow_to_js_error)?;
    let secret_fr = hex_to_fr(&secret).map_err(anyhow_to_js_error)?;
    let leaf_index_u64 = parse_u64(&leaf_index).map_err(anyhow_to_js_error)?;

    let siblings_vec = siblings
        .into_iter()
        .map(|sibling| hex_to_fr(&sibling).map_err(anyhow_to_js_error))
        .collect::<Result<Vec<_>, _>>()?;

    let siblings_arr: [Fr; DEPTH] = siblings_vec
        .try_into()
        .map_err(|_| JsValue::from_str("failed to convert siblings into fixed-size array"))?;

    let circuit = SingleWithdrawCircuit::<Fr, DEPTH> {
        poseidon_params,
        merkle_root: Some(merkle_root_fr),
        recipient: Some(recipient_fr),
        withdraw_value: Some(withdraw_value_fr),
        value: Some(value_fr),
        delta: Some(delta_fr),
        secret: Some(secret_fr),
        leaf_index: Some(leaf_index_u64),
        siblings: siblings_arr.map(Some),
    };

    let public_inputs = circuit
        .public_inputs()
        .map_err(|err| JsValue::from_str(&err.to_string()))?;

    let mut rng = StdRng::seed_from_u64(42);
    let proof_bytes = params
        .generate_proof(&mut rng, circuit, &public_inputs)
        .map_err(|err| JsValue::from_str(&err.to_string()))?;

    log_timing(&format!(
        "SingleWithdrawWasm::prove total {:.2} ms (depth {DEPTH})",
        prove_start.elapsed().as_secs_f64() * 1_000.0
    ));

    let public_inputs_hex = public_inputs.iter().map(fr_to_hex).collect::<Vec<_>>();
    let result = JsGroth16ProveResult {
        proof_calldata: format!("0x{}", hex::encode(proof_bytes)),
        public_inputs: public_inputs_hex,
        tree_depth: DEPTH,
    };

    serde_wasm_bindgen::to_value(&result).map_err(|err| JsValue::from_str(&err.to_string()))
}
