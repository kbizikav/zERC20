mod api;
mod config;
mod fee;
mod oracle;
mod submitter;

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

    let state = web::Data::new(AppState {
        relayer_key,
        tokens: cfg.tokens,
        oracle,
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
            .app_data(state.clone())
            .app_data(json_cfg)
            .route("/relay/teleport", web::post().to(api::relay_teleport))
            .route("/relay/fee-estimate", web::get().to(api::fee_estimate))
    })
    .bind(("0.0.0.0", cfg.port))
    .context("failed to bind relay server")?
    .run()
    .await
    .context("relay server exited with error")?;

    Ok(())
}
