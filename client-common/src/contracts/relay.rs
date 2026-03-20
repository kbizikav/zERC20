#![allow(clippy::too_many_arguments)]

use alloy::{
    primitives::{Address, B256, U256, keccak256},
    signers::{Signer, local::PrivateKeySigner},
    sol,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::contracts::utils::NormalProvider;

// ---------------------------------------------------------------------------
// ABI: EIP-712 domain (shared across Verifier and other EIP-712 contracts)
// ---------------------------------------------------------------------------

sol! {
    #[sol(rpc)]
    interface IEIP712Domain {
        function eip712Domain()
            external
            view
            returns (
                bytes1 fields,
                string memory name,
                string memory version,
                uint256 chainId,
                address verifyingContract,
                bytes32 salt,
                uint256[] memory extensions
            );
    }

    struct GeneralRecipient {
        uint64 chainId;
        bytes32 recipient;
        bytes32 tweak;
    }

    struct RelayerFeeAuthorization {
        uint256 relayerFee;
        uint256 maxFee;
        uint64 deadline;
        bytes signature;
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const EIP712_FIELD_NAME: u8 = 1 << 0;
const EIP712_FIELD_VERSION: u8 = 1 << 1;
const EIP712_FIELD_CHAIN_ID: u8 = 1 << 2;
const EIP712_FIELD_VERIFYING_CONTRACT: u8 = 1 << 3;
const SUPPORTED_EIP712_FIELDS: u8 = EIP712_FIELD_NAME
    | EIP712_FIELD_VERSION
    | EIP712_FIELD_CHAIN_ID
    | EIP712_FIELD_VERIFYING_CONTRACT;

/// EIP-712 type hash used by Verifier._verifyRelayerFeeAuthorization.
fn relayer_fee_typehash() -> B256 {
    keccak256(
        "RelayerFeeAuthorization(uint256 recipientHash,uint256 totalValue,uint256 maxFee,uint64 deadline)",
    )
}

fn ensure_supported_eip712_fields(fields: u8, contract_name: &str) -> Result<()> {
    if fields != SUPPORTED_EIP712_FIELDS {
        bail!(
            "{} eip712Domain() returned unsupported fields bitmask 0x{:02x}; expected 0x{:02x}",
            contract_name,
            fields,
            SUPPORTED_EIP712_FIELDS
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// EIP-712 signature
// ---------------------------------------------------------------------------

/// Computes the EIP-712 domain separator by calling `Verifier.eip712Domain()` (ERC-5267).
///
/// The Verifier inherits OZ EIP712Upgradeable which exposes this view function.
pub async fn fetch_domain_separator(
    provider: NormalProvider,
    verifier_address: Address,
) -> Result<B256> {
    let contract = IEIP712Domain::new(verifier_address, provider);
    let result = contract.eip712Domain().call().await.context(
        "failed to call eip712Domain() on Verifier — ensure Verifier has been upgraded with initializeV2",
    )?;
    ensure_supported_eip712_fields(result.fields.as_slice()[0], "Verifier")?;

    let type_hash = keccak256(
        "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let name_hash = keccak256(result.name.as_bytes());
    let version_hash = keccak256(result.version.as_bytes());

    // Encode: abi.encode(typeHash, nameHash, versionHash, chainId, verifyingContract)
    let mut buf = Vec::with_capacity(5 * 32);
    buf.extend_from_slice(type_hash.as_slice());
    buf.extend_from_slice(name_hash.as_slice());
    buf.extend_from_slice(version_hash.as_slice());
    buf.extend_from_slice(&result.chainId.to_be_bytes::<32>());
    buf.extend_from_slice(B256::left_padding_from(result.verifyingContract.as_slice()).as_slice());

    Ok(keccak256(&buf))
}

/// Signs a `RelayerFeeAuthorization` using EIP-712 typed data.
///
/// Returns the 65-byte `(r, s, v)` signature.
pub async fn sign_relayer_fee_authorization(
    private_key: B256,
    domain_separator: B256,
    recipient_hash: U256,
    total_value: U256,
    max_fee: U256,
    deadline: u64,
) -> Result<Vec<u8>> {
    // struct hash = keccak256(abi.encode(RELAYER_FEE_TYPEHASH, recipientHash, totalValue, maxFee, deadline))
    let mut struct_data = Vec::with_capacity(5 * 32);
    struct_data.extend_from_slice(relayer_fee_typehash().as_slice());
    struct_data.extend_from_slice(&recipient_hash.to_be_bytes::<32>());
    struct_data.extend_from_slice(&total_value.to_be_bytes::<32>());
    struct_data.extend_from_slice(&max_fee.to_be_bytes::<32>());
    // deadline is uint64, ABI-encoded as uint256
    struct_data.extend_from_slice(&U256::from(deadline).to_be_bytes::<32>());

    let struct_hash = keccak256(&struct_data);

    // EIP-712 digest: keccak256("\x19\x01" || domainSeparator || structHash)
    let mut digest_input = Vec::with_capacity(2 + 32 + 32);
    digest_input.extend_from_slice(&[0x19, 0x01]);
    digest_input.extend_from_slice(domain_separator.as_slice());
    digest_input.extend_from_slice(struct_hash.as_slice());
    let digest = keccak256(&digest_input);

    let signer = PrivateKeySigner::from_bytes(&private_key)
        .context("failed to create signer from private key")?;
    let sig = signer
        .sign_hash(&digest)
        .await
        .context("failed to sign relayer fee authorization")?;

    Ok(sig.as_bytes().to_vec())
}

// ---------------------------------------------------------------------------
// Custom relay node HTTP client
// ---------------------------------------------------------------------------

fn relay_http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// Request to submit a teleport via the custom relay node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayTeleportRequest {
    pub is_global: bool,
    pub root_hint: u64,
    pub chain_id: u64,
    pub recipient: B256,
    pub tweak: B256,
    #[serde(with = "hex_vec")]
    pub proof: Vec<u8>,
    pub relayer_fee: U256,
    pub max_fee: U256,
    pub deadline: u64,
    #[serde(with = "hex_vec")]
    pub signature: Vec<u8>,
}

/// Serde helper for `Vec<u8>` encoded as a hex string.
mod hex_vec {
    use serde::{Deserialize, Deserializer, Serializer, de};

    pub fn serialize<S: Serializer>(data: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("0x{}", hex::encode(data)))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(deserializer)?;
        let s = s.strip_prefix("0x").unwrap_or(&s);
        hex::decode(s).map_err(de::Error::custom)
    }
}

/// Submit a teleport request to the custom relay node.
///
/// Returns the transaction hash on success.
pub async fn submit_relay_teleport(relay_url: &str, req: &RelayTeleportRequest) -> Result<B256> {
    let url = format!("{}/relay/teleport", relay_url.trim_end_matches('/'));

    let resp = relay_http_client()
        .post(&url)
        .json(req)
        .send()
        .await
        .context("relay node HTTP request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("relay node returned {}: {}", status, body);
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RelayResponse {
        tx_hash: B256,
    }
    let parsed: RelayResponse = resp
        .json()
        .await
        .context("failed to parse relay node response")?;
    Ok(parsed.tx_hash)
}

/// Fetch a fee estimate from the custom relay node.
pub async fn estimate_relay_fee(relay_url: &str, chain_id: u64) -> Result<U256> {
    let url = format!(
        "{}/relay/fee-estimate?chain_id={}",
        relay_url.trim_end_matches('/'),
        chain_id,
    );

    let resp = relay_http_client()
        .get(&url)
        .send()
        .await
        .context("relay node fee-estimate HTTP request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("relay node fee-estimate returned {}: {}", status, body);
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FeeResponse {
        relayer_fee: U256,
    }
    let parsed: FeeResponse = resp
        .json()
        .await
        .context("failed to parse relay node fee-estimate response")?;
    Ok(parsed.relayer_fee)
}
