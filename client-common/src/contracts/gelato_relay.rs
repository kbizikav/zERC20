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
// ABI: GelatoTeleportRelay (inline sol! definition)
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
    interface IGelatoTeleportRelay {
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

/// EIP-712 type hash used by Verifier._verifyRelayerFeeAuthorization.
fn relayer_fee_typehash() -> B256 {
    keccak256(
        "RelayerFeeAuthorization(uint256 recipientHash,uint256 totalValue,uint256 maxFee,uint64 deadline)",
    )
}

/// Gelato Relay API base URL.
const GELATO_RELAY_URL: &str = "https://relay.gelato.digital";

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

/// Encodes `GelatoTeleportRelay.relayTeleport(...)` calldata.
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
    let call = IGelatoTeleportRelay::relayTeleportCall {
        isGlobal: params.is_global,
        rootHint: params.root_hint,
        gr,
        proof: Bytes::copy_from_slice(&params.proof),
        feeAuth: fee_auth,
        maxGelatoFee: params.max_gelato_fee,
    };
    Bytes::from(call.abi_encode())
}

/// Encodes `GelatoTeleportRelay.relaySingleTeleport(...)` calldata.
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
    let call = IGelatoTeleportRelay::relaySingleTeleportCall {
        isGlobal: params.is_global,
        rootHint: params.root_hint,
        gr,
        proof: Bytes::copy_from_slice(&params.proof),
        feeAuth: fee_auth,
        maxGelatoFee: params.max_gelato_fee,
    };
    Bytes::from(call.abi_encode())
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
            let with_buffer = candidate
                + candidate * U256::from(RELAY_FEE_BUFFER_BPS) / U256::from(BPS_DENOMINATOR);
            return Ok(RelayerFeeEstimate {
                relayer_fee: with_buffer,
                gelato_fee,
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
    let with_buffer =
        candidate + candidate * U256::from(RELAY_FEE_BUFFER_BPS) / U256::from(BPS_DENOMINATOR);
    Ok(RelayerFeeEstimate {
        relayer_fee: with_buffer,
        gelato_fee,
        unwrap_fee: final_unwrap_fee,
    })
}

pub struct RelayerFeeEstimate {
    pub relayer_fee: U256,
    pub gelato_fee: U256,
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
