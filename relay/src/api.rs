use actix_web::{HttpResponse, web};
use alloy::primitives::{Address, B256, U256};

use crate::oracle::PriceOracle;
use crate::submitter;
use client_common::contracts::utils::ProviderWithSigner;
use client_common::contracts::relay::RelayTeleportRequest;
use client_common::tokens::{TokenEntry, TokenType};

/// Shared application state.
pub struct AppState {
    pub relayer_address: Address,
    pub tokens: Vec<TokenEntry>,
    pub signer_providers: std::collections::HashMap<u64, ProviderWithSigner>,
    pub oracle: PriceOracle,
    pub swap_enabled: bool,
    pub swap_fee_bps: u64,
    pub max_swap_native_wei: U256,
}

impl AppState {
    fn find_token(&self, chain_id: u64) -> Option<&TokenEntry> {
        self.tokens.iter().find(|t| t.chain_id == chain_id)
    }

    fn signer_provider(&self, chain_id: u64) -> Option<&ProviderWithSigner> {
        self.signer_providers.get(&chain_id)
    }
}

async fn compute_swap_native_amount(
    state: &AppState,
    token: &TokenEntry,
    token_type: TokenType,
    token_amount: U256,
) -> anyhow::Result<(U256, U256)> {
    let native_before_fee = state
        .oracle
        .convert_token_to_native(token.chain_id, token_type, token_amount)
        .await?;
    let provider = token.provider()?;
    let relayer_fee =
        crate::fee::estimate_native_fee(&provider, crate::fee::SWAP_GAS_LIMIT).await?;
    let native_after_bps =
        native_before_fee * U256::from(10_000 - state.swap_fee_bps) / U256::from(10_000u64);
    Ok((native_after_bps.saturating_sub(relayer_fee), relayer_fee))
}

async fn estimate_swap_quote_from_target_native(
    state: &AppState,
    token: &TokenEntry,
    token_type: TokenType,
    target_native_amount: U256,
) -> anyhow::Result<(U256, U256, U256, bool)> {
    let provider = token.provider()?;
    let relayer_fee =
        crate::fee::estimate_native_fee(&provider, crate::fee::SWAP_GAS_LIMIT).await?;

    let capped = target_native_amount > state.max_swap_native_wei;
    let target_native_amount = if capped {
        state.max_swap_native_wei
    } else {
        target_native_amount
    };

    let denominator = U256::from(10_000u64 - state.swap_fee_bps);
    let numerator = (target_native_amount + relayer_fee) * U256::from(10_000u64);
    let native_before_fee_needed = (numerator + denominator - U256::from(1u64)) / denominator;

    let mut token_amount = state
        .oracle
        .convert_native_to_token(token.chain_id, token_type, native_before_fee_needed)
        .await?;
    if token_amount.is_zero() {
        token_amount = U256::from(1u64);
    }

    let mut native_amount = compute_swap_native_amount(state, token, token_type, token_amount)
        .await?
        .0;

    for _ in 0..4 {
        if native_amount >= target_native_amount || native_amount >= state.max_swap_native_wei {
            break;
        }

        let shortfall = target_native_amount - native_amount;
        let top_up_before_fee =
            (shortfall * U256::from(10_000u64) + denominator - U256::from(1u64)) / denominator;
        let top_up_token_amount = state
            .oracle
            .convert_native_to_token(token.chain_id, token_type, top_up_before_fee)
            .await?;
        token_amount += top_up_token_amount.max(U256::from(1u64));
        native_amount = compute_swap_native_amount(state, token, token_type, token_amount)
            .await?
            .0;
    }

    Ok((token_amount, native_amount, relayer_fee, capped))
}

/// GET /relay/info
pub async fn relay_info(state: web::Data<AppState>) -> HttpResponse {
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
        "address": format!("{}", state.relayer_address),
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

    let provider = match state.signer_provider(req.chain_id) {
        Some(p) => p,
        None => {
            return HttpResponse::InternalServerError().json(
                serde_json::json!({"error": format!("no signer provider configured for chain {}", req.chain_id)}),
            );
        }
    };

    match submitter::submit_teleport(token, provider, &req).await {
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

    let (native_amount, relayer_fee) =
        match compute_swap_native_amount(state.get_ref(), token, token_type, token_amount).await {
            Ok(v) => v,
            Err(err) => {
                return HttpResponse::InternalServerError()
                    .json(serde_json::json!({"error": format!("{:#}", err)}));
            }
        };

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
        Ok(a) if !a.is_zero() => a,
        _ => {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"error": "min_native_amount must be a positive decimal"}));
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

    let (native_amount, relayer_fee) =
        match compute_swap_native_amount(state.get_ref(), token, token_type, token_amount).await {
            Ok(v) => v,
            Err(err) => {
                return HttpResponse::InternalServerError()
                    .json(serde_json::json!({"error": format!("{:#}", err)}));
            }
        };

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
        state.signer_provider(req.chain_id),
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
