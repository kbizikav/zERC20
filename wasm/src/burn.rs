use crate::utils::{
    address_to_hex_string, anyhow_to_js_error, b256_to_hex_string, fr_to_hex_checked,
    normalize_hex, parse_address_hex, parse_b256_hex, serde_error_to_js,
};
use alloy::primitives::Address;
use anyhow::Context;
use client_common::payment::{burn_address::FullBurnAddress, invoice::SecretAndTweak};
use serde::Serialize;
use wasm_bindgen::prelude::*;
use zkp::utils::{convertion::fr_to_b256, general_recipient::GeneralRecipient};

const SEED_MESSAGE: &str = "zERC20 | Seed Derivation\n\nYou are signing to derive a private seed used ONLY to generate\none-time burn receiving addresses for zERC20.\n\nFacts:\n- Not a transaction; no gas or approvals.\n- Cannot move funds or grant permissions.\n- If this signature is exposed, privacy may be reduced\n  (burn addresses may become linkable). Funds remain safe.\n- Keep this signature private.\n\nDetails:\n- App: zERC20\n- Purpose: Seed for burn address derivation\n- Version: 1";

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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::str::FromStr;

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
