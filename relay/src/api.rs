use actix_web::{HttpResponse, web};
use alloy::primitives::{Address, B256, U256};
use alloy::signers::local::PrivateKeySigner;

use crate::oracle::PriceOracle;
use crate::submitter;
use client_common::contracts::relay::RelayTeleportRequest;
use client_common::tokens::TokenEntry;

/// Shared application state.
pub struct AppState {
    pub relayer_key: B256,
    pub tokens: Vec<TokenEntry>,
    pub oracle: PriceOracle,
    pub swap_enabled: bool,
    pub swap_fee_bps: u64,
    pub max_swap_native_wei: U256,
}

impl AppState {
    fn find_token(&self, chain_id: u64) -> Option<&TokenEntry> {
        self.tokens.iter().find(|t| t.chain_id == chain_id)
    }
}

/// GET /relay/info
pub async fn relay_info(state: web::Data<AppState>) -> HttpResponse {
    let signer = PrivateKeySigner::from_bytes(&state.relayer_key).unwrap();
    let address = signer.address();

    // Build chain_id -> swap_helper_address map
    let swap_helper_addresses: std::collections::HashMap<String, String> = state
        .tokens
        .iter()
        .filter_map(|t| {
            t.swap_helper_address
                .map(|addr| (t.chain_id.to_string(), format!("{}", addr)))
        })
        .collect();

    HttpResponse::Ok().json(serde_json::json!({
        "address": format!("{}", address),
        "swapEnabled": state.swap_enabled,
        "swapFeeBps": state.swap_fee_bps,
        "swapHelperAddresses": swap_helper_addresses,
    }))
}

/// POST /relay/teleport
pub async fn relay_teleport(
    state: web::Data<AppState>,
    body: web::Json<RelayTeleportRequest>,
) -> HttpResponse {
    let req = body.into_inner();

    let token = match state.find_token(req.chain_id) {
        Some(t) => t,
        None => {
            return HttpResponse::BadRequest().json(
                serde_json::json!({"error": format!("unsupported chain_id {}", req.chain_id)}),
            );
        }
    };

    // Basic validation
    if req.signature.len() != 65 {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"error": "signature must be 65 bytes"}));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if req.deadline < now {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"error": "deadline has passed"}));
    }

    match submitter::submit_teleport(token, &state.relayer_key, &req).await {
        Ok(tx_hash) => HttpResponse::Ok().json(serde_json::json!({"txHash": tx_hash})),
        Err(err) => {
            log::error!("teleport submission failed: {:?}", err);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("{:#}", err)}))
        }
    }
}

/// Query params for fee estimate.
#[derive(serde::Deserialize)]
pub struct FeeEstimateQuery {
    pub chain_id: u64,
}

/// GET /relay/fee-estimate?chain_id=X
pub async fn fee_estimate(
    state: web::Data<AppState>,
    query: web::Query<FeeEstimateQuery>,
) -> HttpResponse {
    let token = match state.find_token(query.chain_id) {
        Some(t) => t,
        None => {
            return HttpResponse::BadRequest().json(
                serde_json::json!({"error": format!("unsupported chain_id {}", query.chain_id)}),
            );
        }
    };

    let provider = match token.provider() {
        Ok(p) => p,
        Err(err) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("failed to create provider: {}", err)}));
        }
    };

    let token_type = match token.token_type {
        Some(tt) => tt,
        None => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": format!("token_type not configured for chain {}", query.chain_id)
            }));
        }
    };

    match crate::fee::estimate_fee(&provider, token.chain_id, token_type, &state.oracle).await {
        Ok(fee) => HttpResponse::Ok().json(serde_json::json!({"relayerFee": fee.to_string()})),
        Err(err) => {
            log::error!("fee estimation failed: {:?}", err);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("{:#}", err)}))
        }
    }
}

// ---------------------------------------------------------------------------
// Swap endpoints
// ---------------------------------------------------------------------------

/// Query params for swap quote.
#[derive(serde::Deserialize)]
pub struct SwapQuoteQuery {
    pub chain_id: u64,
    /// Token amount in smallest unit (decimal string).
    pub amount: String,
}

/// GET /relay/swap-quote?chain_id=X&amount=Y
pub async fn swap_quote(
    state: web::Data<AppState>,
    query: web::Query<SwapQuoteQuery>,
) -> HttpResponse {
    if !state.swap_enabled {
        return HttpResponse::ServiceUnavailable()
            .json(serde_json::json!({"error": "swap is not enabled on this relay"}));
    }

    let token = match state.find_token(query.chain_id) {
        Some(t) => t,
        None => {
            return HttpResponse::BadRequest().json(
                serde_json::json!({"error": format!("unsupported chain_id {}", query.chain_id)}),
            );
        }
    };

    let token_type = match token.token_type {
        Some(tt) => tt,
        None => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": format!("token_type not configured for chain {}", query.chain_id)
            }));
        }
    };

    let token_amount = match U256::from_str_radix(&query.amount, 10) {
        Ok(a) if !a.is_zero() => a,
        _ => {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"error": "amount must be a positive decimal integer"}));
        }
    };

    let native_before_fee = match state
        .oracle
        .convert_token_to_native(query.chain_id, token_type, token_amount)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("{:#}", err)}));
        }
    };

    // Apply fee: native_amount = native_before_fee * (10000 - fee_bps) / 10000
    let fee_bps = state.swap_fee_bps;
    let native_amount = native_before_fee * U256::from(10_000 - fee_bps) / U256::from(10_000u64);

    if native_amount > state.max_swap_native_wei {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": format!("swap output {} exceeds max {}", native_amount, state.max_swap_native_wei)
        }));
    }

    HttpResponse::Ok().json(serde_json::json!({
        "nativeAmount": native_amount.to_string(),
        "feeBps": fee_bps,
    }))
}

/// Request body for swap execution.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapRequest {
    pub chain_id: u64,
    /// Token amount to swap (decimal string).
    pub token_amount: String,
    /// Minimum native output the user will accept (slippage protection).
    pub min_native_amount: String,
    /// Address to receive native tokens.
    pub recipient: Address,
    /// The address that signed the permit (token owner).
    pub owner: Address,
    pub permit_deadline: String,
    pub permit_v: u8,
    pub permit_r: B256,
    pub permit_s: B256,
}

/// POST /relay/swap
pub async fn relay_swap(state: web::Data<AppState>, body: web::Json<SwapRequest>) -> HttpResponse {
    if !state.swap_enabled {
        return HttpResponse::ServiceUnavailable()
            .json(serde_json::json!({"error": "swap is not enabled on this relay"}));
    }

    let req = body.into_inner();

    let token = match state.find_token(req.chain_id) {
        Some(t) => t,
        None => {
            return HttpResponse::BadRequest().json(
                serde_json::json!({"error": format!("unsupported chain_id {}", req.chain_id)}),
            );
        }
    };

    let token_type = match token.token_type {
        Some(tt) => tt,
        None => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": format!("token_type not configured for chain {}", req.chain_id)
            }));
        }
    };

    let token_amount = match U256::from_str_radix(&req.token_amount, 10) {
        Ok(a) if !a.is_zero() => a,
        _ => {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"error": "token_amount must be a positive decimal"}));
        }
    };

    let min_native = match U256::from_str_radix(&req.min_native_amount, 10) {
        Ok(a) => a,
        _ => {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"error": "min_native_amount must be a valid decimal"}));
        }
    };

    let permit_deadline = match U256::from_str_radix(&req.permit_deadline, 10) {
        Ok(d) => d,
        _ => {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"error": "permit_deadline must be a valid decimal"}));
        }
    };

    // Check deadline
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if permit_deadline < U256::from(now) {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"error": "permit deadline has passed"}));
    }

    // Compute native output
    let native_before_fee = match state
        .oracle
        .convert_token_to_native(req.chain_id, token_type, token_amount)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("{:#}", err)}));
        }
    };

    let native_amount =
        native_before_fee * U256::from(10_000 - state.swap_fee_bps) / U256::from(10_000u64);

    // Slippage protection
    if native_amount < min_native {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": format!(
                "computed native output {} is below min_native_amount {}",
                native_amount, min_native
            )
        }));
    }

    // Max swap check
    if native_amount > state.max_swap_native_wei {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": format!("swap output {} exceeds max {}", native_amount, state.max_swap_native_wei)
        }));
    }

    match submitter::submit_swap(
        token,
        &state.relayer_key,
        req.owner,
        req.recipient,
        token_amount,
        native_amount,
        permit_deadline,
        req.permit_v,
        req.permit_r,
        req.permit_s,
    )
    .await
    {
        Ok(tx_hash) => HttpResponse::Ok().json(serde_json::json!({
            "txHash": format!("{}", tx_hash),
        })),
        Err(err) => {
            log::error!("swap submission failed: {:?}", err);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("{:#}", err)}))
        }
    }
}
