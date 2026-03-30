use actix_web::{HttpResponse, web};
use alloy::primitives::{Address, B256, U256};
use alloy::signers::local::PrivateKeySigner;

use crate::fee::GasFeeCache;
use crate::oracle::PriceOracle;
use crate::submitter;
use client_common::contracts::relay::RelayTeleportRequest;
use client_common::tokens::{TokenEntry, TokenType};

/// Shared application state.
pub struct AppState {
    pub relayer_key: B256,
    pub tokens: Vec<TokenEntry>,
    pub oracle: PriceOracle,
    pub swap_enabled: bool,
    pub swap_fee_bps: u64,
    pub max_swap_native_wei: U256,
    pub gas_fee_cache: GasFeeCache,
}

impl AppState {
    fn find_token(&self, chain_id: u64) -> Option<&TokenEntry> {
        self.tokens.iter().find(|t| t.chain_id == chain_id)
    }
}

/// Compute the native output for a given token amount, using a pre-computed relayer fee.
fn compute_swap_native_output(
    swap_fee_bps: u64,
    native_before_fee: U256,
    relayer_fee: U256,
) -> U256 {
    let native_after_bps =
        native_before_fee * U256::from(10_000 - swap_fee_bps) / U256::from(10_000u64);
    native_after_bps.saturating_sub(relayer_fee)
}

/// Estimate the relayer fee for swaps on the given token's chain, using the cache.
async fn cached_swap_relayer_fee(state: &AppState, token: &TokenEntry) -> anyhow::Result<U256> {
    let provider = token.provider()?;
    state
        .gas_fee_cache
        .get_or_estimate(token.chain_id, &provider, crate::fee::SWAP_GAS_LIMIT)
        .await
}

async fn estimate_swap_quote_from_target_native(
    state: &AppState,
    token: &TokenEntry,
    token_type: TokenType,
    target_native_amount: U256,
) -> anyhow::Result<(U256, U256, U256, bool)> {
    let relayer_fee = cached_swap_relayer_fee(state, token).await?;

    let capped = target_native_amount > state.max_swap_native_wei;
    let target_native_amount = if capped {
        state.max_swap_native_wei
    } else {
        target_native_amount
    };

    let denominator = U256::from(10_000u64 - state.swap_fee_bps);
    let numerator = (target_native_amount + relayer_fee) * U256::from(10_000u64);
    let native_before_fee_needed = (numerator + denominator - U256::from(1u64)) / denominator;

    // Convert to token units, adding 1 to compensate for integer truncation in
    // the oracle's division so the forward calculation never falls short.
    let token_amount = state
        .oracle
        .convert_native_to_token(token.chain_id, token_type, native_before_fee_needed)
        .await?
        + U256::from(1u64);

    let native_before_fee = state
        .oracle
        .convert_token_to_native(token.chain_id, token_type, token_amount)
        .await?;
    let native_amount =
        compute_swap_native_output(state.swap_fee_bps, native_before_fee, relayer_fee);

    Ok((token_amount, native_amount, relayer_fee, capped))
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
        "maxSwapNativeWei": state.max_swap_native_wei.to_string(),
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

/// Query params for target-output swap quote.
#[derive(serde::Deserialize)]
pub struct SwapQuoteTargetQuery {
    pub chain_id: u64,
    /// Desired native output in wei (decimal string).
    pub target_native_amount: String,
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

    let relayer_fee = match cached_swap_relayer_fee(state.get_ref(), token).await {
        Ok(f) => f,
        Err(err) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("{:#}", err)}));
        }
    };

    let native_before_fee = match state
        .oracle
        .convert_token_to_native(token.chain_id, token_type, token_amount)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("{:#}", err)}));
        }
    };

    let native_amount =
        compute_swap_native_output(state.swap_fee_bps, native_before_fee, relayer_fee);

    if native_amount.is_zero() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": format!(
                "swap output is zero after deducting relayer fee {}",
                relayer_fee
            )
        }));
    }

    let fee_bps = state.swap_fee_bps;

    if native_amount > state.max_swap_native_wei {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": format!("swap output {} exceeds max {}", native_amount, state.max_swap_native_wei)
        }));
    }

    HttpResponse::Ok().json(serde_json::json!({
        "nativeAmount": native_amount.to_string(),
        "feeBps": fee_bps,
        "relayerFee": relayer_fee.to_string(),
    }))
}

/// GET /relay/swap-quote-target?chain_id=X&target_native_amount=Y
pub async fn swap_quote_target(
    state: web::Data<AppState>,
    query: web::Query<SwapQuoteTargetQuery>,
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

    let target_native_amount = match U256::from_str_radix(&query.target_native_amount, 10) {
        Ok(a) if !a.is_zero() => a,
        _ => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": "target_native_amount must be a positive decimal integer"
            }));
        }
    };

    let (token_amount, native_amount, relayer_fee, capped_to_max) =
        match estimate_swap_quote_from_target_native(
            state.get_ref(),
            token,
            token_type,
            target_native_amount,
        )
        .await
        {
            Ok(v) => v,
            Err(err) => {
                return HttpResponse::InternalServerError()
                    .json(serde_json::json!({"error": format!("{:#}", err)}));
            }
        };

    HttpResponse::Ok().json(serde_json::json!({
        "tokenAmount": token_amount.to_string(),
        "nativeAmount": native_amount.to_string(),
        "feeBps": state.swap_fee_bps,
        "relayerFee": relayer_fee.to_string(),
        "requestedNativeAmount": target_native_amount.to_string(),
        "maxNativeAmount": state.max_swap_native_wei.to_string(),
        "cappedToMax": capped_to_max,
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

    let relayer_fee = match cached_swap_relayer_fee(state.get_ref(), token).await {
        Ok(f) => f,
        Err(err) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("{:#}", err)}));
        }
    };

    let native_before_fee = match state
        .oracle
        .convert_token_to_native(token.chain_id, token_type, token_amount)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("{:#}", err)}));
        }
    };

    let native_amount =
        compute_swap_native_output(state.swap_fee_bps, native_before_fee, relayer_fee);

    if native_amount.is_zero() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": format!(
                "swap output is zero after deducting relayer fee {}",
                relayer_fee
            )
        }));
    }

    // Slippage protection
    if native_amount < min_native {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": format!(
                "computed native output {} is below min_native_amount {} after deducting relayer fee {}",
                native_amount, min_native, relayer_fee
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
