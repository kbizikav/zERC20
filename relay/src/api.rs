use actix_web::{HttpResponse, web};
use alloy::primitives::B256;

use crate::{config::ChainConfig, submitter};
use client_common::contracts::relay::RelayTeleportRequest;

/// Shared application state.
pub struct AppState {
    pub relayer_key: B256,
    pub chains: Vec<ChainConfig>,
}

impl AppState {
    fn find_chain(&self, chain_id: u64) -> Option<&ChainConfig> {
        self.chains.iter().find(|c| c.chain_id == chain_id)
    }
}

/// POST /relay/teleport
pub async fn relay_teleport(
    state: web::Data<AppState>,
    body: web::Json<RelayTeleportRequest>,
) -> HttpResponse {
    let req = body.into_inner();

    let chain = match state.find_chain(req.chain_id) {
        Some(c) => c,
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

    match submitter::submit_teleport(chain, &state.relayer_key, &req).await {
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
    let chain = match state.find_chain(query.chain_id) {
        Some(c) => c,
        None => {
            return HttpResponse::BadRequest().json(
                serde_json::json!({"error": format!("unsupported chain_id {}", query.chain_id)}),
            );
        }
    };

    let rpc_url = match chain.rpc_url.parse() {
        Ok(url) => url,
        Err(err) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("invalid RPC URL: {}", err)}));
        }
    };

    let provider = alloy::providers::ProviderBuilder::new().connect_http(rpc_url);

    match crate::fee::estimate_fee(&provider, chain).await {
        Ok(fee) => HttpResponse::Ok().json(serde_json::json!({"relayerFee": fee})),
        Err(err) => {
            log::error!("fee estimation failed: {:?}", err);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("{:#}", err)}))
        }
    }
}
