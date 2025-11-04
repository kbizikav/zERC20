use alloy::primitives::{Address, B256, U256};
use anyhow::Context;
use ark_bn254::Fr;
use ark_ff::PrimeField;
use ark_serialize::CanonicalSerialize;
use client_common::payment::{burn_address::FullBurnAddress, invoice::SecretAndTweak};
use folding_schemes::FoldingScheme;
use serde::{Deserialize, Serialize};
use std::convert::TryInto;
use std::str::FromStr;
use wasm_bindgen::prelude::*;
use web_time::Instant;
use zkp::{
    groth16::{params::Groth16Params, withdraw::SingleWithdrawCircuit},
    nova::{
        constants::{AGGREGATION_TREE_HEIGHT, GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
        params::NovaParams,
        withdraw_nova::{WITHDRAW_STATE_LEN, WithdrawCircuit, WithdrawExternalInputs},
    },
    utils::{
        convertion::{fr_to_b256, u256_to_fr},
        general_recipient::GeneralRecipient,
        poseidon::utils::circom_poseidon_config,
        tree::merkle_tree::MerkleTree,
    },
};

use rand::{SeedableRng, rngs::StdRng};

const SEED_MESSAGE: &str = "zERC20 | Seed Derivation\n\nYou are signing to derive a private seed used ONLY to generate\none-time burn receiving addresses for zERC20.\n\nFacts:\n- Not a transaction; no gas or approvals.\n- Cannot move funds or grant permissions.\n- If this signature is exposed, privacy may be reduced\n  (burn addresses may become linkable). Funds remain safe.\n- Keep this signature private.\n\nDetails:\n- App: zERC20\n- Purpose: Seed for burn address derivation\n- Version: 1";

#[derive(Debug, Serialize, Deserialize)]
pub struct JsExternalInput {
    #[serde(default)]
    pub is_dummy: bool,
    pub value: String,
    pub secret: String,
    #[serde(rename = "leafIndex")]
    pub leaf_index: String,
    pub siblings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct JsProveResult {
    #[serde(rename = "finalState")]
    final_state: Vec<String>,
    #[serde(rename = "ivcProof")]
    ivc_proof: String,
    steps: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsSingleWithdrawInput {
    #[serde(rename = "merkleRoot")]
    pub merkle_root: String,
    pub recipient: String,
    #[serde(rename = "withdrawValue")]
    pub withdraw_value: String,
    pub value: String,
    pub delta: String,
    pub secret: String,
    #[serde(rename = "leafIndex")]
    pub leaf_index: String,
    pub siblings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct JsGroth16ProveResult {
    #[serde(rename = "proofCalldata")]
    proof_calldata: String,
    #[serde(rename = "publicInputs")]
    public_inputs: Vec<String>,
    #[serde(rename = "treeDepth")]
    tree_depth: usize,
}

#[derive(Debug, Serialize)]
struct JsSecretTweak {
    secret: String,
    tweak: String,
}

#[derive(Debug, Serialize)]
struct JsGeneralRecipient {
    #[serde(rename = "chainId")]
    chain_id: u64,
    address: String,
    tweak: String,
    #[serde(rename = "fr")]
    fr: String,
    #[serde(rename = "u256")]
    u256: String,
}

#[derive(Debug, Serialize)]
struct JsBurnArtifacts {
    #[serde(rename = "burnAddress")]
    burn_address: String,
    #[serde(rename = "fullBurnAddress")]
    full_burn_address: String,
    #[serde(rename = "generalRecipient")]
    general_recipient: JsGeneralRecipient,
    #[serde(rename = "secret")]
    secret: String,
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

fn parse_u64(value: &str) -> anyhow::Result<u64> {
    let normalized = value.trim();
    if normalized.is_empty() {
        anyhow::bail!("empty numeric value");
    }
    if let Some(hex) = normalized.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).context("invalid hex for u64")
    } else {
        normalized.parse::<u64>().context("invalid decimal for u64")
    }
}

pub fn hex_to_fr(value: &str) -> anyhow::Result<Fr> {
    let normalized = value.trim();
    if normalized.is_empty() {
        anyhow::bail!("empty field element");
    }
    let hex = normalized.strip_prefix("0x").unwrap_or(normalized);
    let bytes = hex::decode(hex).context("invalid hex field element")?;
    Ok(Fr::from_be_bytes_mod_order(&bytes))
}

pub fn fr_to_hex(value: &Fr) -> String {
    use ark_ff::BigInteger;

    let mut bytes = value.into_bigint().to_bytes_be();
    if bytes.len() < 32 {
        let mut padded = vec![0u8; 32 - bytes.len()];
        padded.extend_from_slice(&bytes);
        bytes = padded;
    }
    format!("0x{}", hex::encode(bytes))
}

fn anyhow_to_js_error(err: anyhow::Error) -> JsValue {
    JsValue::from_str(&err.to_string())
}

fn serde_error_to_js(err: serde_wasm_bindgen::Error) -> JsValue {
    JsValue::from_str(&err.to_string())
}

#[cfg(target_arch = "wasm32")]
pub fn log_timing(message: &str) {
    web_sys::console::log_1(&JsValue::from_str(message));
}

#[cfg(not(target_arch = "wasm32"))]
pub fn log_timing(_message: &str) {}

fn normalize_hex(value: &str) -> &str {
    value.strip_prefix("0x").unwrap_or(value)
}

fn parse_b256_hex(value: &str) -> anyhow::Result<B256> {
    let hex = normalize_hex(value);
    let bytes = hex::decode(hex).context("invalid hex value")?;
    anyhow::ensure!(
        bytes.len() == 32,
        "expected 32-byte value, found {} bytes",
        bytes.len()
    );
    Ok(B256::from_slice(&bytes))
}

fn parse_address_hex(value: &str) -> anyhow::Result<Address> {
    Address::from_str(value).context("invalid recipient address")
}

fn b256_to_hex_string(value: B256) -> String {
    format!("0x{}", hex::encode(value.as_slice()))
}

fn address_to_hex_string(value: Address) -> String {
    format!("{:#x}", value)
}

fn fr_to_hex_checked(value: Fr) -> String {
    fr_to_hex(&value)
}

fn secret_tweak_to_js(secret_and_tweak: SecretAndTweak) -> JsSecretTweak {
    JsSecretTweak {
        secret: b256_to_hex_string(secret_and_tweak.secret),
        tweak: b256_to_hex_string(secret_and_tweak.tweak),
    }
}

fn general_recipient_to_js(gr: &GeneralRecipient) -> JsGeneralRecipient {
    let fr_value = gr.to_fr();
    let fr_hex = fr_to_hex_checked(fr_value);
    let u256_hex = b256_to_hex_string(fr_to_b256(fr_value));
    let recipient_address = Address::from_word(gr.address);
    JsGeneralRecipient {
        chain_id: gr.chain_id,
        address: address_to_hex_string(recipient_address),
        tweak: b256_to_hex_string(gr.tweak),
        fr: fr_hex,
        u256: u256_hex,
    }
}

fn build_burn_artifacts(burn: &FullBurnAddress) -> anyhow::Result<JsBurnArtifacts> {
    let burn_address = burn
        .burn_address()
        .context("failed to derive burn address from payload")?;
    Ok(JsBurnArtifacts {
        burn_address: address_to_hex_string(burn_address),
        full_burn_address: format!("0x{}", hex::encode(burn.to_bytes())),
        general_recipient: general_recipient_to_js(&burn.gr),
        secret: b256_to_hex_string(burn.secret),
    })
}

#[wasm_bindgen]
pub fn seed_message() -> String {
    SEED_MESSAGE.to_string()
}

#[wasm_bindgen]
pub fn derive_payment_advice(
    seed_hex: &str,
    payment_advice_id_hex: &str,
    recipient_chain_id: u64,
    recipient_address_hex: &str,
) -> Result<JsValue, JsValue> {
    console_error_panic_hook::set_once();
    let seed = parse_b256_hex(seed_hex).map_err(anyhow_to_js_error)?;
    let payment_advice_id = parse_b256_hex(payment_advice_id_hex).map_err(anyhow_to_js_error)?;
    let recipient_address = parse_address_hex(recipient_address_hex).map_err(anyhow_to_js_error)?;
    let secret_and_tweak = SecretAndTweak::payment_advice(
        payment_advice_id,
        seed,
        recipient_chain_id,
        recipient_address,
    );
    let result = secret_tweak_to_js(secret_and_tweak);
    serde_wasm_bindgen::to_value(&result).map_err(serde_error_to_js)
}

#[wasm_bindgen]
pub fn derive_invoice_single(
    seed_hex: &str,
    invoice_id_hex: &str,
    recipient_chain_id: u64,
    recipient_address_hex: &str,
) -> Result<JsValue, JsValue> {
    console_error_panic_hook::set_once();
    let seed = parse_b256_hex(seed_hex).map_err(anyhow_to_js_error)?;
    let invoice_id = parse_b256_hex(invoice_id_hex).map_err(anyhow_to_js_error)?;
    let recipient_address = parse_address_hex(recipient_address_hex).map_err(anyhow_to_js_error)?;
    let secret_and_tweak =
        SecretAndTweak::single_invoice(invoice_id, seed, recipient_chain_id, recipient_address);
    let result = secret_tweak_to_js(secret_and_tweak);
    serde_wasm_bindgen::to_value(&result).map_err(serde_error_to_js)
}

#[wasm_bindgen]
pub fn derive_invoice_batch(
    seed_hex: &str,
    invoice_id_hex: &str,
    sub_id: u32,
    recipient_chain_id: u64,
    recipient_address_hex: &str,
) -> Result<JsValue, JsValue> {
    console_error_panic_hook::set_once();
    let seed = parse_b256_hex(seed_hex).map_err(anyhow_to_js_error)?;
    let invoice_id = parse_b256_hex(invoice_id_hex).map_err(anyhow_to_js_error)?;
    let recipient_address = parse_address_hex(recipient_address_hex).map_err(anyhow_to_js_error)?;
    let secret_and_tweak = SecretAndTweak::batch_invoice(
        invoice_id,
        sub_id,
        seed,
        recipient_chain_id,
        recipient_address,
    );
    let result = secret_tweak_to_js(secret_and_tweak);
    serde_wasm_bindgen::to_value(&result).map_err(serde_error_to_js)
}

#[wasm_bindgen]
pub fn build_full_burn_address(
    recipient_chain_id: u64,
    recipient_address_hex: &str,
    secret_hex: &str,
    tweak_hex: &str,
) -> Result<JsValue, JsValue> {
    console_error_panic_hook::set_once();
    let recipient_address = parse_address_hex(recipient_address_hex).map_err(anyhow_to_js_error)?;
    let secret = parse_b256_hex(secret_hex).map_err(anyhow_to_js_error)?;
    let tweak = parse_b256_hex(tweak_hex).map_err(anyhow_to_js_error)?;
    let secret_and_tweak = SecretAndTweak { secret, tweak };
    let burn = FullBurnAddress::new(recipient_chain_id, recipient_address, &secret_and_tweak)
        .map_err(anyhow_to_js_error)?;
    let artifacts = build_burn_artifacts(&burn).map_err(anyhow_to_js_error)?;
    serde_wasm_bindgen::to_value(&artifacts).map_err(serde_error_to_js)
}

#[wasm_bindgen]
pub fn decode_full_burn_address(full_burn_address_hex: &str) -> Result<JsValue, JsValue> {
    console_error_panic_hook::set_once();
    let payload_hex = normalize_hex(full_burn_address_hex);
    let bytes = hex::decode(payload_hex)
        .context("invalid FullBurnAddress hex")
        .map_err(anyhow_to_js_error)?;
    let burn = FullBurnAddress::from_bytes(&bytes).map_err(anyhow_to_js_error)?;
    let artifacts = build_burn_artifacts(&burn).map_err(anyhow_to_js_error)?;
    serde_wasm_bindgen::to_value(&artifacts).map_err(serde_error_to_js)
}

#[wasm_bindgen]
pub fn general_recipient_fr(
    chain_id: u64,
    recipient_address_hex: &str,
    tweak_hex: &str,
) -> Result<String, JsValue> {
    console_error_panic_hook::set_once();
    let recipient_address = parse_address_hex(recipient_address_hex).map_err(anyhow_to_js_error)?;
    let tweak = parse_b256_hex(tweak_hex).map_err(anyhow_to_js_error)?;
    let gr = GeneralRecipient::new_evm(chain_id, recipient_address, tweak);
    Ok(fr_to_hex_checked(gr.to_fr()))
}

fn parse_u256_hex(value: &str) -> anyhow::Result<U256> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty hex value");
    }
    let normalized = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    U256::from_str_radix(normalized, 16).context("invalid hex-encoded U256")
}

#[wasm_bindgen]
pub fn aggregation_root(snapshot_hex: JsValue) -> Result<String, JsValue> {
    console_error_panic_hook::set_once();
    let snapshot: Vec<String> = serde_wasm_bindgen::from_value(snapshot_hex)
        .map_err(|err| JsValue::from_str(&err.to_string()))?;
    let mut tree = MerkleTree::new(AGGREGATION_TREE_HEIGHT);
    for (idx, hex) in snapshot.iter().enumerate() {
        let value = match parse_u256_hex(hex) {
            Ok(val) => val,
            Err(err) => return Err(anyhow_to_js_error(err)),
        };
        if value.is_zero() {
            continue;
        }
        tree.update_leaf(idx as u64, u256_to_fr(value));
    }
    Ok(fr_to_hex(&tree.get_root()))
}

#[wasm_bindgen]
pub fn aggregation_merkle_proof(snapshot_hex: JsValue, index: u32) -> Result<JsValue, JsValue> {
    console_error_panic_hook::set_once();
    let snapshot: Vec<String> = serde_wasm_bindgen::from_value(snapshot_hex)
        .map_err(|err| JsValue::from_str(&err.to_string()))?;
    let mut tree = MerkleTree::new(AGGREGATION_TREE_HEIGHT);
    for (idx, hex) in snapshot.iter().enumerate() {
        let value = match parse_u256_hex(hex) {
            Ok(val) => val,
            Err(err) => return Err(anyhow_to_js_error(err)),
        };
        if value.is_zero() {
            continue;
        }
        tree.update_leaf(idx as u64, u256_to_fr(value));
    }
    if index as usize >= (1usize << AGGREGATION_TREE_HEIGHT) {
        return Err(JsValue::from_str("aggregation index exceeds tree capacity"));
    }
    let proof = tree.prove(index as u64);
    let siblings: Vec<String> = proof.siblings.iter().map(fr_to_hex).collect();
    serde_wasm_bindgen::to_value(&siblings).map_err(|err| JsValue::from_str(&err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    #[test]
    fn secret_tweak_to_js_formats_hex() {
        let seed = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let advice = "0xabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd";
        let chain_id = 1u64;
        let recipient_address =
            Address::from_str("0x0000000000000000000000000000000000000001").unwrap();

        let secret_and_tweak = SecretAndTweak::payment_advice(
            parse_b256_hex(advice).expect("advice hex"),
            parse_b256_hex(seed).expect("seed hex"),
            chain_id,
            recipient_address,
        );
        let js_value = secret_tweak_to_js(secret_and_tweak);

        assert_eq!(js_value.secret, b256_to_hex_string(secret_and_tweak.secret));
        assert_eq!(js_value.tweak, b256_to_hex_string(secret_and_tweak.tweak));
        assert!(js_value.secret.starts_with("0x"));
        assert!(js_value.tweak.starts_with("0x"));
        assert_eq!(js_value.secret.len(), 66);
        assert_eq!(js_value.tweak.len(), 66);
    }

    #[test]
    fn build_burn_artifacts_match_full_burn_address() -> Result<()> {
        let seed =
            parse_b256_hex("0x2222222222222222222222222222222222222222222222222222222222222222")
                .unwrap();
        let invoice =
            parse_b256_hex("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .unwrap();
        let recipient_address =
            Address::from_str("0x1111111111111111111111111111111111111111").unwrap();
        let chain_id = 42161u64;
        let secret_and_tweak =
            SecretAndTweak::single_invoice(invoice, seed, chain_id, recipient_address);

        let burn = FullBurnAddress::new(chain_id, recipient_address, &secret_and_tweak)?;
        let artifacts = build_burn_artifacts(&burn)?;
        let expected_burn_address = address_to_hex_string(burn.burn_address()?);

        assert_eq!(artifacts.burn_address, expected_burn_address);
        assert_eq!(
            artifacts.full_burn_address,
            format!("0x{}", hex::encode(burn.to_bytes()))
        );
        assert_eq!(artifacts.secret, b256_to_hex_string(burn.secret));
        assert_eq!(artifacts.general_recipient.chain_id, chain_id);
        assert_eq!(
            artifacts.general_recipient.tweak,
            b256_to_hex_string(secret_and_tweak.tweak)
        );
        assert_eq!(
            artifacts.general_recipient.fr,
            fr_to_hex_checked(burn.gr.to_fr())
        );
        Ok(())
    }

    #[test]
    fn full_burn_address_round_trip() -> Result<()> {
        let seed =
            parse_b256_hex("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
                .unwrap();
        let invoice =
            parse_b256_hex("0x9999999999999999999999999999999999999999999999999999999999999999")
                .unwrap();
        let recipient_address =
            Address::from_str("0x8888888888888888888888888888888888888888").unwrap();
        let chain_id = 10u64;
        let secret_and_tweak =
            SecretAndTweak::single_invoice(invoice, seed, chain_id, recipient_address);

        let burn = FullBurnAddress::new(chain_id, recipient_address, &secret_and_tweak)?;
        let encoded = burn.to_bytes();
        let decoded = FullBurnAddress::from_bytes(&encoded)?;

        assert_eq!(decoded.gr.chain_id, burn.gr.chain_id);
        assert_eq!(decoded.gr.address, burn.gr.address);
        assert_eq!(decoded.gr.tweak, burn.gr.tweak);
        assert_eq!(decoded.secret, burn.secret);
        Ok(())
    }
}
