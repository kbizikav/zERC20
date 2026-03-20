use actix_web::{HttpResponse, web};
use alloy::primitives::B256;

use crate::oracle::PriceOracle;
use crate::submitter;
use client_common::contracts::relay::RelayTeleportRequest;
use client_common::tokens::TokenEntry;

/// Shared application state.
pub struct AppState {
    pub relayer_key: B256,
    pub tokens: Vec<TokenEntry>,
    pub oracle: PriceOracle,
}

impl AppState {
    fn find_token(&self, chain_id: u64) -> Option<&TokenEntry> {
        self.tokens.iter().find(|t| t.chain_id == chain_id)
    }
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
        Ok(fee) => HttpResponse::Ok().json(serde_json::json!({"relayerFee": fee})),
        Err(err) => {
            log::error!("fee estimation failed: {:?}", err);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("{:#}", err)}))
        }
    }
}
