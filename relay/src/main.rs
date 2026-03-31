// sol! macro generates functions with many args for EVM ABI bindings.
#![allow(clippy::too_many_arguments)]

mod api;
mod config;
mod fee;
mod oracle;
mod submitter;

use actix_cors::Cors;
use actix_web::{App, HttpResponse, HttpServer, web};
use alloy::primitives::{Address, B256};
use anyhow::{Context, Result};
use std::collections::HashMap;

use api::AppState;
use client_common::contracts::utils::{get_address_from_private_key, get_provider_with_signer};

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
    let key_bytes = hex::decode(normalized).context("failed to decode RELAYER_PRIVATE_KEY hex")?;
    anyhow::ensure!(
        key_bytes.len() == 32,
        "RELAYER_PRIVATE_KEY must be 32 bytes"
    );
    let relayer_key = B256::from_slice(&key_bytes);
    let relayer_address: Address = get_address_from_private_key(relayer_key);

    let oracle = oracle::PriceOracle::new(&cfg.tokens, cfg.oracle_rpc_url.as_deref())
        .context("failed to initialize price oracle")?;

    let mut signer_providers = HashMap::new();
    for token in &cfg.tokens {
        signer_providers
            .entry(token.chain_id)
            .or_insert(get_provider_with_signer(
                &token.provider().with_context(|| {
                    format!(
                        "failed to create signer provider for chain {} ({})",
                        token.chain_id, token.label
                    )
                })?,
                relayer_key,
            ));
    }

    if cfg.swap_enabled {
        log::info!(
            "Swap enabled: fee={}bps, max_native={:?}",
            cfg.swap_fee_bps,
            cfg.max_swap_native_wei
        );
        for t in &cfg.tokens {
            match t.swap_helper_address {
                Some(addr) => {
                    log::info!("  SwapHelper on chain {}: {}", t.chain_id, addr);
                }
                None => {
                    anyhow::bail!(
                        "SWAP_ENABLED=true but chain {} ({}) has no swap_helper_address configured",
                        t.chain_id,
                        t.label
                    );
                }
            }
        }
    }

    let state = web::Data::new(AppState {
        relayer_address,
        tokens: cfg.tokens,
        signer_providers,
        oracle,
        swap_enabled: cfg.swap_enabled,
        swap_fee_bps: cfg.swap_fee_bps,
        max_swap_native_wei: cfg.max_swap_native_wei,
        gas_fee_cache: fee::GasFeeCache::new(),
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
            .route(
                "/relay/swap-quote-target",
                web::get().to(api::swap_quote_target),
            )
            .route("/relay/swap", web::post().to(api::relay_swap))
    })
    .bind(("0.0.0.0", cfg.port))
    .context("failed to bind relay server")?
    .run()
    .await
    .context("relay server exited with error")?;

    Ok(())
}
