#![allow(clippy::too_many_arguments)]

use alloy::{
    primitives::{Address, B256, Bytes, U256, keccak256},
    signers::{Signer, local::PrivateKeySigner},
    sol,
    sol_types::SolCall,
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::contracts::{liquidity_manager::LiquidityManagerContract, utils::NormalProvider};

// ---------------------------------------------------------------------------
// ABI: GelatoRelay (inline sol! definition)
// ---------------------------------------------------------------------------

sol! {
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

    #[sol(rpc)]
    interface IGelatoRelay {
        function relayTeleport(
            bool isGlobal,
            uint64 rootHint,
            GeneralRecipient calldata gr,
            bytes calldata proof,
            RelayerFeeAuthorization calldata feeAuth,
            uint256 maxGelatoFee
        ) external;

        function relaySingleTeleport(
            bool isGlobal,
            uint64 rootHint,
            GeneralRecipient calldata gr,
            bytes calldata proof,
            RelayerFeeAuthorization calldata feeAuth,
            uint256 maxGelatoFee
        ) external;

        function relayUnwrap(
            address owner,
            uint256 amount,
            address receiver,
            uint256 relayerFee,
            uint256 maxGelatoFee,
            uint256 deadline,
            bytes calldata permitSig,
            bytes calldata relaySig
        ) external;

        function relayTransfer(
            address owner,
            address to,
            uint256 amount,
            uint256 relayerFee,
            uint256 maxGelatoFee,
            uint256 deadline,
            bytes calldata permitSig,
            bytes calldata relaySig
        ) external;

        function nonces(address owner) external view returns (uint256);
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Gas overhead added by Gelato relay infrastructure.
const GELATO_RELAY_GAS_OVERHEAD: u64 = 150_000;
/// Default gas limit for the teleport call itself.
const DEFAULT_GAS_LIMIT: u64 = 1_000_000;
/// 5 % safety buffer in basis points.
const RELAY_FEE_BUFFER_BPS: u64 = 500;
const BPS_DENOMINATOR: u64 = 10_000;

fn apply_relay_fee_buffer(amount: U256) -> U256 {
    amount + amount * U256::from(RELAY_FEE_BUFFER_BPS) / U256::from(BPS_DENOMINATOR)
}

/// EIP-712 type hash used by Verifier._verifyRelayerFeeAuthorization.
fn relayer_fee_typehash() -> B256 {
    keccak256(
        "RelayerFeeAuthorization(uint256 recipientHash,uint256 totalValue,uint256 maxFee,uint64 deadline)",
    )
}

/// Gelato Relay API base URL.
const GELATO_RELAY_URL: &str = "https://relay.gelato.digital";
const EIP712_FIELD_NAME: u8 = 1 << 0;
const EIP712_FIELD_VERSION: u8 = 1 << 1;
const EIP712_FIELD_CHAIN_ID: u8 = 1 << 2;
const EIP712_FIELD_VERIFYING_CONTRACT: u8 = 1 << 3;
const SUPPORTED_EIP712_FIELDS: u8 =
    EIP712_FIELD_NAME | EIP712_FIELD_VERSION | EIP712_FIELD_CHAIN_ID | EIP712_FIELD_VERIFYING_CONTRACT;

/// Default number of polls when waiting for a relay task.
const DEFAULT_POLLS: u32 = 40;
/// Default interval between polls in milliseconds.
const DEFAULT_INTERVAL_MS: u64 = 3_000;

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
    // EIP-5267: eip712Domain() returns (fields, name, version, chainId, verifyingContract, salt, extensions)
    // We compute the domain separator from these values.
    sol! {
        #[sol(rpc)]
        interface IEIP712 {
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
    }

    let contract = IEIP712::new(verifier_address, provider);
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
// Calldata encoding
// ---------------------------------------------------------------------------

/// Parameters for a relay teleport call.
pub struct RelayTeleportParams {
    pub is_global: bool,
    pub root_hint: u64,
    pub chain_id: u64,
    pub recipient: B256,
    pub tweak: B256,
    pub proof: Vec<u8>,
    pub relayer_fee: U256,
    pub max_fee: U256,
    pub deadline: u64,
    pub signature: Vec<u8>,
    pub max_gelato_fee: U256,
}

/// Encodes `GelatoRelay.relayTeleport(...)` calldata.
pub fn encode_relay_teleport(params: &RelayTeleportParams) -> Bytes {
    let gr = GeneralRecipient {
        chainId: params.chain_id,
        recipient: params.recipient,
        tweak: params.tweak,
    };
    let fee_auth = RelayerFeeAuthorization {
        relayerFee: params.relayer_fee,
        maxFee: params.max_fee,
        deadline: params.deadline,
        signature: Bytes::from(params.signature.clone()),
    };
    let call = IGelatoRelay::relayTeleportCall {
        isGlobal: params.is_global,
        rootHint: params.root_hint,
        gr,
        proof: Bytes::copy_from_slice(&params.proof),
        feeAuth: fee_auth,
        maxGelatoFee: params.max_gelato_fee,
    };
    Bytes::from(call.abi_encode())
}

/// Encodes `GelatoRelay.relaySingleTeleport(...)` calldata.
pub fn encode_relay_single_teleport(params: &RelayTeleportParams) -> Bytes {
    let gr = GeneralRecipient {
        chainId: params.chain_id,
        recipient: params.recipient,
        tweak: params.tweak,
    };
    let fee_auth = RelayerFeeAuthorization {
        relayerFee: params.relayer_fee,
        maxFee: params.max_fee,
        deadline: params.deadline,
        signature: Bytes::from(params.signature.clone()),
    };
    let call = IGelatoRelay::relaySingleTeleportCall {
        isGlobal: params.is_global,
        rootHint: params.root_hint,
        gr,
        proof: Bytes::copy_from_slice(&params.proof),
        feeAuth: fee_auth,
        maxGelatoFee: params.max_gelato_fee,
    };
    Bytes::from(call.abi_encode())
}

/// Parameters for a relay unwrap call.
pub struct RelayUnwrapParams {
    pub owner: Address,
    pub amount: U256,
    pub receiver: Address,
    pub relayer_fee: U256,
    pub max_gelato_fee: U256,
    pub deadline: U256,
    pub permit_sig: Vec<u8>,
    pub relay_sig: Vec<u8>,
}

/// Encodes `GelatoRelay.relayUnwrap(...)` calldata.
pub fn encode_relay_unwrap(params: &RelayUnwrapParams) -> Bytes {
    let call = IGelatoRelay::relayUnwrapCall {
        owner: params.owner,
        amount: params.amount,
        receiver: params.receiver,
        relayerFee: params.relayer_fee,
        maxGelatoFee: params.max_gelato_fee,
        deadline: params.deadline,
        permitSig: Bytes::from(params.permit_sig.clone()),
        relaySig: Bytes::from(params.relay_sig.clone()),
    };
    Bytes::from(call.abi_encode())
}

/// Parameters for a relay transfer call.
pub struct RelayTransferParams {
    pub owner: Address,
    pub to: Address,
    pub amount: U256,
    pub relayer_fee: U256,
    pub max_gelato_fee: U256,
    pub deadline: U256,
    pub permit_sig: Vec<u8>,
    pub relay_sig: Vec<u8>,
}

/// Encodes `GelatoRelay.relayTransfer(...)` calldata.
pub fn encode_relay_transfer(params: &RelayTransferParams) -> Bytes {
    let call = IGelatoRelay::relayTransferCall {
        owner: params.owner,
        to: params.to,
        amount: params.amount,
        relayerFee: params.relayer_fee,
        maxGelatoFee: params.max_gelato_fee,
        deadline: params.deadline,
        permitSig: Bytes::from(params.permit_sig.clone()),
        relaySig: Bytes::from(params.relay_sig.clone()),
    };
    Bytes::from(call.abi_encode())
}

/// Fetches the GelatoRelay contract's EIP-712 domain separator (for relay operation signatures).
pub async fn fetch_relay_domain_separator(
    provider: NormalProvider,
    relay_address: Address,
) -> Result<B256> {
    sol! {
        #[sol(rpc)]
        interface IEIP712 {
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
    }

    let contract = IEIP712::new(relay_address, provider);
    let result = contract.eip712Domain().call().await.context(
        "failed to call eip712Domain() on GelatoRelay — ensure initializeV2 has been called",
    )?;
    ensure_supported_eip712_fields(result.fields.as_slice()[0], "GelatoRelay")?;

    let type_hash = keccak256(
        "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let name_hash = keccak256(result.name.as_bytes());
    let version_hash = keccak256(result.version.as_bytes());

    let mut buf = Vec::with_capacity(5 * 32);
    buf.extend_from_slice(type_hash.as_slice());
    buf.extend_from_slice(name_hash.as_slice());
    buf.extend_from_slice(version_hash.as_slice());
    buf.extend_from_slice(&result.chainId.to_be_bytes::<32>());
    buf.extend_from_slice(B256::left_padding_from(result.verifyingContract.as_slice()).as_slice());

    Ok(keccak256(&buf))
}

/// Fetches the current relay nonce for `owner` on the GelatoRelay contract.
pub async fn fetch_relay_nonce(
    provider: NormalProvider,
    relay_address: Address,
    owner: Address,
) -> Result<U256> {
    let contract = IGelatoRelay::new(relay_address, provider);
    let nonce = contract
        .nonces(owner)
        .call()
        .await
        .context("failed to fetch relay nonce")?;
    Ok(nonce)
}

fn relay_unwrap_typehash() -> B256 {
    keccak256(
        "RelayUnwrap(address owner,uint256 amount,address receiver,uint256 relayerFee,uint256 maxGelatoFee,uint256 nonce)",
    )
}

fn relay_transfer_typehash() -> B256 {
    keccak256(
        "RelayTransfer(address owner,address to,uint256 amount,uint256 relayerFee,uint256 maxGelatoFee,uint256 nonce)",
    )
}

/// Signs a `RelayUnwrap` EIP-712 typed data message.
///
/// Returns the 65-byte `(r, s, v)` signature.
pub async fn sign_relay_unwrap(
    private_key: B256,
    domain_separator: B256,
    owner: Address,
    amount: U256,
    receiver: Address,
    relayer_fee: U256,
    max_gelato_fee: U256,
    nonce: U256,
) -> Result<Vec<u8>> {
    let mut struct_data = Vec::with_capacity(7 * 32);
    struct_data.extend_from_slice(relay_unwrap_typehash().as_slice());
    struct_data.extend_from_slice(B256::left_padding_from(owner.as_slice()).as_slice());
    struct_data.extend_from_slice(&amount.to_be_bytes::<32>());
    struct_data.extend_from_slice(B256::left_padding_from(receiver.as_slice()).as_slice());
    struct_data.extend_from_slice(&relayer_fee.to_be_bytes::<32>());
    struct_data.extend_from_slice(&max_gelato_fee.to_be_bytes::<32>());
    struct_data.extend_from_slice(&nonce.to_be_bytes::<32>());
    let struct_hash = keccak256(&struct_data);

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
        .context("failed to sign relay unwrap authorization")?;

    Ok(sig.as_bytes().to_vec())
}

/// Signs a `RelayTransfer` EIP-712 typed data message.
///
/// Returns the 65-byte `(r, s, v)` signature.
pub async fn sign_relay_transfer(
    private_key: B256,
    domain_separator: B256,
    owner: Address,
    to: Address,
    amount: U256,
    relayer_fee: U256,
    max_gelato_fee: U256,
    nonce: U256,
) -> Result<Vec<u8>> {
    let mut struct_data = Vec::with_capacity(7 * 32);
    struct_data.extend_from_slice(relay_transfer_typehash().as_slice());
    struct_data.extend_from_slice(B256::left_padding_from(owner.as_slice()).as_slice());
    struct_data.extend_from_slice(B256::left_padding_from(to.as_slice()).as_slice());
    struct_data.extend_from_slice(&amount.to_be_bytes::<32>());
    struct_data.extend_from_slice(&relayer_fee.to_be_bytes::<32>());
    struct_data.extend_from_slice(&max_gelato_fee.to_be_bytes::<32>());
    struct_data.extend_from_slice(&nonce.to_be_bytes::<32>());
    let struct_hash = keccak256(&struct_data);

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
        .context("failed to sign relay transfer authorization")?;

    Ok(sig.as_bytes().to_vec())
}

/// Signs an ERC-2612 permit using EIP-712 typed data.
///
/// Calls `eip712Domain()` on the zERC20 token to obtain the domain separator,
/// then signs the Permit struct hash. Returns `(v, r, s)`.
pub async fn sign_permit(
    private_key: B256,
    provider: NormalProvider,
    token_address: Address,
    spender: Address,
    value: U256,
    nonce: U256,
    deadline: U256,
) -> Result<(u8, B256, B256)> {
    // Fetch EIP-712 domain from the zERC20 token
    sol! {
        #[sol(rpc)]
        interface IERC20PermitDomain {
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

            function nonces(address owner) external view returns (uint256);
        }
    }

    let contract = IERC20PermitDomain::new(token_address, provider);
    let domain = contract
        .eip712Domain()
        .call()
        .await
        .context("failed to call eip712Domain() on zERC20 token")?;
    ensure_supported_eip712_fields(domain.fields.as_slice()[0], "zERC20")?;

    let domain_type_hash = keccak256(
        "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let name_hash = keccak256(domain.name.as_bytes());
    let version_hash = keccak256(domain.version.as_bytes());

    let mut domain_buf = Vec::with_capacity(5 * 32);
    domain_buf.extend_from_slice(domain_type_hash.as_slice());
    domain_buf.extend_from_slice(name_hash.as_slice());
    domain_buf.extend_from_slice(version_hash.as_slice());
    domain_buf.extend_from_slice(&domain.chainId.to_be_bytes::<32>());
    domain_buf
        .extend_from_slice(B256::left_padding_from(domain.verifyingContract.as_slice()).as_slice());
    let domain_separator = keccak256(&domain_buf);

    // Permit struct hash
    let permit_typehash = keccak256(
        "Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)",
    );
    let signer = PrivateKeySigner::from_bytes(&private_key)
        .context("failed to create signer from private key")?;
    let owner = signer.address();

    let mut struct_data = Vec::with_capacity(6 * 32);
    struct_data.extend_from_slice(permit_typehash.as_slice());
    struct_data.extend_from_slice(B256::left_padding_from(owner.as_slice()).as_slice());
    struct_data.extend_from_slice(B256::left_padding_from(spender.as_slice()).as_slice());
    struct_data.extend_from_slice(&value.to_be_bytes::<32>());
    struct_data.extend_from_slice(&nonce.to_be_bytes::<32>());
    struct_data.extend_from_slice(&deadline.to_be_bytes::<32>());
    let struct_hash = keccak256(&struct_data);

    // EIP-712 digest
    let mut digest_input = Vec::with_capacity(2 + 32 + 32);
    digest_input.extend_from_slice(&[0x19, 0x01]);
    digest_input.extend_from_slice(domain_separator.as_slice());
    digest_input.extend_from_slice(struct_hash.as_slice());
    let digest = keccak256(&digest_input);

    let sig = signer
        .sign_hash(&digest)
        .await
        .context("failed to sign ERC-2612 permit")?;

    let sig_bytes = sig.as_bytes();
    // sig_bytes is [r(32) | s(32) | v(1)]
    let r = B256::from_slice(&sig_bytes[..32]);
    let s = B256::from_slice(&sig_bytes[32..64]);
    let v = sig_bytes[64];

    Ok((v, r, s))
}

/// Fetches the current ERC-2612 nonce for `owner` on the given token.
pub async fn fetch_permit_nonce(
    provider: NormalProvider,
    token_address: Address,
    owner: Address,
) -> Result<U256> {
    sol! {
        #[sol(rpc)]
        interface IERC20Nonces {
            function nonces(address owner) external view returns (uint256);
        }
    }
    let contract = IERC20Nonces::new(token_address, provider);
    let nonce = contract
        .nonces(owner)
        .call()
        .await
        .context("failed to fetch permit nonce")?;
    Ok(nonce)
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
// Gelato REST API client
// ---------------------------------------------------------------------------

/// Estimate the relayer fee (in zERC20) that covers Gelato gas cost + unwrap fee + 5 % buffer.
///
/// Mirrors the TypeScript SDK `estimateRelayerFee()` logic.
pub async fn estimate_relayer_fee(
    chain_id: u64,
    fee_token: Address,
    gas_limit: Option<u64>,
    liquidity_manager: &LiquidityManagerContract,
) -> Result<RelayerFeeEstimate> {
    let gas_limit = gas_limit.unwrap_or(DEFAULT_GAS_LIMIT);
    let total_gas_limit = gas_limit + GELATO_RELAY_GAS_OVERHEAD;

    // Fetch Gelato gas price oracle estimate
    let gelato_fee = estimate_gelato_fee(chain_id, fee_token, total_gas_limit)
        .await
        .context("failed to estimate Gelato fee via oracle API")?;

    // Iteratively find how much zERC20 to unwrap so that net underlying >= gelato_fee
    let mut candidate = gelato_fee;
    for _ in 0..5 {
        let unwrap_fee = liquidity_manager
            .quote_unwrap_fee(candidate)
            .await
            .context("failed to quote unwrap fee")?;
        let net_out = candidate.saturating_sub(unwrap_fee);
        if net_out >= gelato_fee {
            return Ok(RelayerFeeEstimate {
                relayer_fee: apply_relay_fee_buffer(candidate),
                gelato_fee,
                max_gelato_fee: apply_relay_fee_buffer(gelato_fee),
                unwrap_fee,
            });
        }
        candidate = gelato_fee + unwrap_fee + U256::from(1);
    }

    // Fallback with generous buffer
    let final_unwrap_fee = liquidity_manager
        .quote_unwrap_fee(candidate)
        .await
        .context("failed to quote final unwrap fee")?;
    Ok(RelayerFeeEstimate {
        relayer_fee: apply_relay_fee_buffer(candidate),
        gelato_fee,
        max_gelato_fee: apply_relay_fee_buffer(gelato_fee),
        unwrap_fee: final_unwrap_fee,
    })
}

pub struct RelayerFeeEstimate {
    pub relayer_fee: U256,
    pub gelato_fee: U256,
    pub max_gelato_fee: U256,
    pub unwrap_fee: U256,
}

/// Call the Gelato oracle to estimate gas cost in `fee_token` units.
async fn estimate_gelato_fee(chain_id: u64, fee_token: Address, gas_limit: u64) -> Result<U256> {
    let url = format!(
        "{}/oracles/{}/estimate?paymentToken={}&gasLimit={}",
        GELATO_RELAY_URL, chain_id, fee_token, gas_limit,
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .context("Gelato oracle HTTP request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("Gelato oracle returned {}: {}", status, body);
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct OracleResponse {
        estimated_fee: String,
    }
    let parsed: OracleResponse = resp
        .json()
        .await
        .context("failed to parse Gelato oracle response")?;
    let fee_str = parsed.estimated_fee.trim();
    let fee = if let Some(hex_str) = fee_str.strip_prefix("0x") {
        U256::from_str_radix(hex_str, 16)
    } else {
        U256::from_str_radix(fee_str, 10)
    }
    .context("failed to parse Gelato estimated fee")?;
    Ok(fee)
}

/// Submit a relay task via Gelato `callWithSyncFee` API.
pub async fn submit_relay_task(
    chain_id: u64,
    target: Address,
    data: &Bytes,
    fee_token: Address,
    api_key: Option<&str>,
    gas_limit: Option<u64>,
) -> Result<String> {
    let total_gas_limit = gas_limit.unwrap_or(DEFAULT_GAS_LIMIT) + GELATO_RELAY_GAS_OVERHEAD;

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CallWithSyncFeeRequest {
        chain_id: u64,
        target: String,
        data: String,
        fee_token: String,
        is_relay_context: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        gas_limit: Option<String>,
    }

    let request = CallWithSyncFeeRequest {
        chain_id,
        target: format!("{}", target),
        data: format!("0x{}", hex::encode(data.as_ref())),
        fee_token: format!("{}", fee_token),
        is_relay_context: true,
        gas_limit: Some(total_gas_limit.to_string()),
    };

    let url = format!("{}/relays/v2/call-with-sync-fee", GELATO_RELAY_URL);
    let client = reqwest::Client::new();
    let mut req_builder = client.post(&url).json(&request);
    if let Some(key) = api_key {
        req_builder = req_builder.header("x-gelato-api-key", key);
    }

    let resp = req_builder
        .send()
        .await
        .context("Gelato callWithSyncFee HTTP request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("Gelato callWithSyncFee returned {}: {}", status, body);
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RelayResponse {
        task_id: String,
    }
    let parsed: RelayResponse = resp
        .json()
        .await
        .context("failed to parse Gelato relay response")?;
    Ok(parsed.task_id)
}

/// Possible terminal states of a Gelato relay task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayTaskState {
    CheckPending,
    ExecPending,
    WaitingForConfirmation,
    ExecSuccess,
    ExecReverted,
    Cancelled,
}

impl RelayTaskState {
    fn from_str(s: &str) -> Self {
        match s {
            "ExecSuccess" => Self::ExecSuccess,
            "ExecReverted" => Self::ExecReverted,
            "Cancelled" => Self::Cancelled,
            "ExecPending" => Self::ExecPending,
            "WaitingForConfirmation" => Self::WaitingForConfirmation,
            _ => Self::CheckPending,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::ExecSuccess | Self::ExecReverted | Self::Cancelled
        )
    }
}

/// Result from polling a Gelato relay task.
#[derive(Debug, Clone)]
pub struct RelayTaskResult {
    pub task_id: String,
    pub task_state: RelayTaskState,
    pub transaction_hash: Option<String>,
    pub last_check_message: Option<String>,
}

/// Poll Gelato for task status until it reaches a terminal state or times out.
pub async fn poll_relay_task(
    task_id: &str,
    polls: Option<u32>,
    interval_ms: Option<u64>,
) -> Result<RelayTaskResult> {
    let polls = polls.unwrap_or(DEFAULT_POLLS);
    let interval_ms = interval_ms.unwrap_or(DEFAULT_INTERVAL_MS);

    let client = reqwest::Client::new();
    let url = format!("{}/tasks/status/{}", GELATO_RELAY_URL, task_id);

    for i in 0..polls {
        let resp = client
            .get(&url)
            .send()
            .await
            .context("Gelato task status HTTP request failed")?;

        if resp.status().is_success() {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct TaskStatusResponse {
                task: Option<TaskStatus>,
            }
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct TaskStatus {
                task_id: String,
                task_state: String,
                transaction_hash: Option<String>,
                last_check_message: Option<String>,
            }

            let parsed: TaskStatusResponse = resp
                .json()
                .await
                .context("failed to parse Gelato task status response")?;

            if let Some(task) = parsed.task {
                let state = RelayTaskState::from_str(&task.task_state);
                if state.is_terminal() {
                    return Ok(RelayTaskResult {
                        task_id: task.task_id,
                        task_state: state,
                        transaction_hash: task.transaction_hash,
                        last_check_message: task.last_check_message,
                    });
                }
            }
        }

        if i < polls - 1 {
            tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
        }
    }

    Ok(RelayTaskResult {
        task_id: task_id.to_string(),
        task_state: RelayTaskState::CheckPending,
        transaction_hash: None,
        last_check_message: Some(format!("Polling timed out after {} attempts", polls)),
    })
}
