use std::time::Instant;

use anyhow::{Context, Result};
use log::{debug, error, warn};
use sqlx::PgPool;

use crate::{
    config::EventJobConfig,
    events::{EventIndexer, EventIndexerConfig},
};
use client_common::{
    contracts::{
        utils::{get_provider, get_provider_with_fallback},
        z_erc20::ZErc20Contract,
    },
    tokens::{TokenEntry, TokenMetadata},
};

use super::try_acquire_lock;

const EVENT_LOCK_SALT: u64 = 0x45564e54; // "EVNT"

#[derive(Clone)]
pub struct EventSyncJob {
    pool: PgPool,
    tokens: Vec<EventTokenContext>,
    interval_ms: u64,
    indexer_config: EventIndexerConfig,
}

impl EventSyncJob {
    pub async fn run_forever(&self) -> Result<()> {
        loop {
            let iteration_started = Instant::now();
            self.run_once().await;
            let elapsed = iteration_started.elapsed();
            let interval = std::time::Duration::from_millis(self.interval_ms);
            if elapsed < interval {
                tokio::time::sleep(interval - elapsed).await;
            }
        }
    }

    pub async fn run_once(&self) {
        for token in &self.tokens {
            if let Err(err) = self.process_token(token).await {
                error!("event sync failed for token '{}': {err:?}", token.label);
            }
        }
    }

    async fn process_token(&self, token: &EventTokenContext) -> Result<()> {
        let Some(lease) = try_acquire_lock(&self.pool, token.lock_key).await? else {
            debug!(
                "skip event sync for '{}' due to lock contention",
                token.label
            );
            return Ok(());
        };

        let sync_result = self.sync_token(token).await;

        if let Err(err) = lease.release().await {
            warn!(
                "failed to release lease for token '{}': {err:?}",
                token.label
            );
        }

        sync_result
    }

    async fn sync_token(&self, token: &EventTokenContext) -> Result<()> {
        let indexer = EventIndexer::new(
            token.contract.clone(),
            self.pool.clone(),
            token.deployed_block_number,
            token.metadata.clone(),
            self.indexer_config,
        )
        .await
        .with_context(|| format!("failed to initialise event indexer for '{}'", token.label))?;

        indexer
            .sync()
            .await
            .with_context(|| format!("event sync failed for '{}'", token.label))?;

        debug!("event sync completed for '{}'", token.label);

        Ok(())
    }
}

pub struct EventSyncJobBuilder {
    pool: PgPool,
    job_config: EventJobConfig,
    tokens: Vec<TokenEntry>,
}

impl EventSyncJobBuilder {
    pub fn new(pool: PgPool, job_config: EventJobConfig, tokens: Vec<TokenEntry>) -> Self {
        Self {
            pool,
            job_config,
            tokens,
        }
    }

    pub fn into_job(self) -> Result<EventSyncJob> {
        let indexer_config = self
            .job_config
            .build_indexer_config()
            .context("invalid event indexer configuration")?;

        let mut contexts = Vec::with_capacity(self.tokens.len());
        for token in self.tokens {
            let provider = if token.rpc_urls.len() == 1 {
                get_provider(token.rpc_urls.first().expect("rpc urls not empty")).with_context(
                    || format!("failed to build provider for token '{}'", token.label),
                )?
            } else {
                get_provider_with_fallback(&token.rpc_urls).with_context(|| {
                    format!(
                        "failed to build fallback provider for token '{}'",
                        token.label
                    )
                })?
            };

            let contract = ZErc20Contract::new(provider, token.token_address)
                .with_legacy_tx(token.legacy_tx);
            let lock_key = token.lock_key_with_salt(EVENT_LOCK_SALT);
            let metadata = token.metadata();
            let context = EventTokenContext {
                label: token.label.clone(),
                metadata,
                deployed_block_number: token.deployed_block_number,
                contract,
                lock_key,
            };
            contexts.push(context);
        }

        Ok(EventSyncJob {
            pool: self.pool,
            tokens: contexts,
            interval_ms: self.job_config.interval_ms,
            indexer_config,
        })
    }
}

#[derive(Clone)]
struct EventTokenContext {
    label: String,
    metadata: TokenMetadata,
    deployed_block_number: u64,
    contract: ZErc20Contract,
    lock_key: i64,
}
