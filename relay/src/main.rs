// sol! macro generates functions with many args for EVM ABI bindings.
#![allow(clippy::too_many_arguments)]

mod api;
mod config;
mod fee;
mod oracle;
mod submitter;

use actix_cors::Cors;
use actix_web::{App, HttpResponse, HttpServer, web};
use alloy::primitives::B256;
use anyhow::{Context, Result};

use api::AppState;

#[actix_web::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    env_logger::init();

    let cfg = config::RelayConfig::from_env().context("failed to load relay configuration")?;

    let normalized = cfg
        .private_key
        .trim()
        .strip_prefix("0x")
        .unwrap_or(cfg.private_key.trim());
    let key_bytes = hex::decode(normalized).context("failed to decode RELAY_PRIVATE_KEY hex")?;
    anyhow::ensure!(key_bytes.len() == 32, "RELAY_PRIVATE_KEY must be 32 bytes");
    let relayer_key = B256::from_slice(&key_bytes);

    let oracle =
        oracle::PriceOracle::new(&cfg.tokens).context("failed to initialize price oracle")?;

    if cfg.swap_enabled {
        log::info!(
            "Swap enabled: fee={}bps, max_native={:?}",
            cfg.swap_fee_bps,
            cfg.max_swap_native_wei
        );
        for t in &cfg.tokens {
            if let Some(addr) = t.swap_helper_address {
                log::info!("  SwapHelper on chain {}: {}", t.chain_id, addr);
            }
        }
    }

    let state = web::Data::new(AppState {
        relayer_key,
        tokens: cfg.tokens,
        oracle,
        swap_enabled: cfg.swap_enabled,
        swap_fee_bps: cfg.swap_fee_bps,
        max_swap_native_wei: cfg.max_swap_native_wei,
    });

    log::info!("Starting relay node on port {}", cfg.port);

    HttpServer::new(move || {
        let json_cfg = web::JsonConfig::default().error_handler(|err, _req| {
            log::error!("JSON deserialization error: {err}");
            let response =
                HttpResponse::BadRequest().json(serde_json::json!({"error": format!("{err}")}));
            actix_web::error::InternalError::from_response(err, response).into()
        });
        App::new()
            // NOTE: Permissive CORS is acceptable for internal use only.
            // If exposing this API externally, restrict origins appropriately.
            .wrap(Cors::permissive())
            .app_data(state.clone())
            .app_data(json_cfg)
            .route("/relay/info", web::get().to(api::relay_info))
            .route("/relay/teleport", web::post().to(api::relay_teleport))
            .route("/relay/fee-estimate", web::get().to(api::fee_estimate))
            .route("/relay/swap-quote", web::get().to(api::swap_quote))
            .route("/relay/swap-quote-target", web::get().to(api::swap_quote_target))
            .route("/relay/swap", web::post().to(api::relay_swap))
    })
    .bind(("0.0.0.0", cfg.port))
    .context("failed to bind relay server")?
    .run()
    .await
    .context("relay server exited with error")?;

    Ok(())
}
