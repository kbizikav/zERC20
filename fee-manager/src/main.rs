use std::{env, path::Path, time::Duration};

use alloy::primitives::{B256, U256};
use anyhow::{Context, Result, bail};
use clap::Parser;
use client_common::{
    contracts::{
        erc20::Erc20Contract,
        liquidity_manager::LiquidityManagerContract,
        utils::{NormalProvider, get_provider, get_provider_with_fallback},
    },
    tokens::{TokenEntry, load_tokens_from_compressed, load_tokens_from_path},
};
use log::{error, info, warn};
use tokio::time::{self, MissedTickBehavior};

#[derive(Parser, Debug)]
#[command(
    name = "fee-manager",
    about = "Periodically updates targetLiquidity on LiquidityManager contracts across chains"
)]
struct Cli {
    /// Tokens configuration file path.
    #[arg(
        long,
        env = "TOKENS_FILE_PATH",
        value_name = "PATH",
        default_value = "../config/tokens.json"
    )]
    tokens_file_path: std::path::PathBuf,

    /// Private key used to submit setFeeParams transactions.
    #[arg(
        long,
        env = "FEE_MANAGER_PRIVATE_KEY",
        value_name = "HEX",
        required = true
    )]
    private_key: String,

    /// Interval in seconds between fee parameter updates.
    #[arg(
        long,
        env = "FEE_MANAGER_INTERVAL_SECS",
        value_name = "SECONDS",
        default_value_t = 3600
    )]
    interval_secs: u64,

    /// Incentive coefficient k in basis points (1 = 0.01%).
    #[arg(
        long,
        env = "FEE_MANAGER_K_BPS",
        value_name = "BPS",
        default_value_t = 1000
    )]
    k_bps: u64,

    /// Run the job once and exit.
    #[arg(long, env = "JOB_ONCE", default_value_t = false)]
    once: bool,
}

/// Per-chain context for fee management operations.
struct ChainContext {
    label: String,
    #[allow(dead_code)]
    chain_id: u64,
    liquidity_manager: LiquidityManagerContract,
    underlying_token: Erc20Contract,
}

/// Main job that updates fee parameters across all chains.
struct FeeManagerJob {
    chains: Vec<ChainContext>,
    private_key: B256,
    interval: Duration,
    k_bps: u64,
}

impl FeeManagerJob {
    async fn run(self) {
        // Execute once immediately
        if let Err(err) = self.execute_once().await {
            error!("initial fee parameter update failed: {err:?}");
        }

        let mut ticker = time::interval(self.interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            ticker.tick().await;
            if let Err(err) = self.execute_once().await {
                error!("fee parameter update failed: {err:?}");
            }
        }
    }

    async fn execute_once(&self) -> Result<()> {
        info!("Fetching underlying balances across {} chains...", self.chains.len());

        // Step 1: Fetch balances from all chains
        let mut balances: Vec<(&ChainContext, U256)> = Vec::with_capacity(self.chains.len());
        let mut total_balance = U256::ZERO;

        for chain in &self.chains {
            match self.fetch_balance(chain).await {
                Ok(balance) => {
                    info!(
                        "[{}] underlying balance: {} (raw)",
                        chain.label, balance
                    );
                    total_balance = total_balance.saturating_add(balance);
                    balances.push((chain, balance));
                }
                Err(err) => {
                    error!(
                        "[{}] failed to fetch underlying balance: {err:?}",
                        chain.label
                    );
                    // Continue with other chains
                }
            }
        }

        if balances.is_empty() {
            warn!("No balances fetched successfully; skipping fee parameter update");
            return Ok(());
        }

        // Step 2: Calculate target liquidity per chain
        let chain_count = U256::from(balances.len());
        let target_liquidity = total_balance / chain_count;
        let k = U256::from(self.k_bps);

        info!(
            "Total liquidity: {} across {} chains",
            total_balance,
            balances.len()
        );
        info!("Target per chain: {} (k: {} bps)", target_liquidity, self.k_bps);

        // Step 3: Update fee params on each chain
        for (chain, _balance) in balances {
            match self.update_fee_params(chain, target_liquidity, k).await {
                Ok(tx_hash) => {
                    info!(
                        "[{}] setFeeParams tx confirmed: {:?}",
                        chain.label, tx_hash
                    );
                }
                Err(err) => {
                    error!(
                        "[{}] failed to update fee params: {err:?}",
                        chain.label
                    );
                    // Continue with other chains
                }
            }
        }

        info!(
            "Fee parameter update complete. Next update in {} seconds.",
            self.interval.as_secs()
        );
        Ok(())
    }

    async fn fetch_balance(&self, chain: &ChainContext) -> Result<U256> {
        let balance = chain
            .underlying_token
            .balance_of(chain.liquidity_manager.address())
            .await
            .with_context(|| {
                format!(
                    "failed to fetch balance for {} at {:?}",
                    chain.label,
                    chain.liquidity_manager.address()
                )
            })?;
        Ok(balance)
    }

    async fn update_fee_params(
        &self,
        chain: &ChainContext,
        target_liquidity: U256,
        k: U256,
    ) -> Result<B256> {
        info!(
            "[{}] Updating feeParams (target: {}, k: {})",
            chain.label, target_liquidity, k
        );

        let pending = chain
            .liquidity_manager
            .set_fee_params(self.private_key, target_liquidity, k)
            .await
            .with_context(|| format!("failed to submit setFeeParams for {}", chain.label))?;

        let receipt = pending
            .get_receipt()
            .await
            .with_context(|| format!("failed to fetch receipt for {}", chain.label))?;

        if !receipt.status() {
            bail!(
                "setFeeParams transaction reverted for {}: {:?}",
                chain.label,
                receipt
            );
        }

        Ok(receipt.transaction_hash)
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    env_logger::init();

    let cli = Cli::parse();

    if cli.interval_secs == 0 {
        bail!("FEE_MANAGER_INTERVAL_SECS must be greater than zero");
    }

    let tokens = load_tokens_config(&cli.tokens_file_path)?;

    // Filter tokens that have a liquidity_manager_address
    let tokens_with_lm: Vec<_> = tokens
        .into_iter()
        .filter(|t| t.liquidity_manager_address.is_some())
        .collect();

    if tokens_with_lm.is_empty() {
        bail!(
            "no tokens with liquidity_manager_address configured; set TOKENS_COMPRESSED or populate {}",
            cli.tokens_file_path.display()
        );
    }

    info!(
        "Found {} chains with LiquidityManager configured",
        tokens_with_lm.len()
    );

    let private_key = parse_private_key(&cli.private_key)?;

    // Build chain contexts
    let mut chains = Vec::with_capacity(tokens_with_lm.len());
    for token in &tokens_with_lm {
        match build_chain_context(token).await {
            Ok(ctx) => {
                info!(
                    "[{}] LiquidityManager: {:?}, Underlying: {:?}",
                    ctx.label,
                    ctx.liquidity_manager.address(),
                    ctx.underlying_token.address()
                );
                chains.push(ctx);
            }
            Err(err) => {
                error!(
                    "failed to build chain context for '{}': {err:?}",
                    token.label
                );
                // Continue with other chains
            }
        }
    }

    if chains.is_empty() {
        bail!("no valid chain contexts could be built");
    }

    let job = FeeManagerJob {
        chains,
        private_key,
        interval: Duration::from_secs(cli.interval_secs),
        k_bps: cli.k_bps,
    };

    if cli.once {
        job.execute_once()
            .await
            .context("fee parameter update failed in --once mode")?;
        return Ok(());
    }

    info!(
        "fee-manager started; updating every {} seconds (Ctrl+C to stop)",
        cli.interval_secs
    );

    let handle = tokio::spawn(async move {
        job.run().await;
    });

    tokio::signal::ctrl_c()
        .await
        .context("failed to listen for Ctrl+C")?;
    info!("Ctrl+C received, shutting down");

    handle.abort();

    Ok(())
}

async fn build_chain_context(token: &TokenEntry) -> Result<ChainContext> {
    let lm_address = token
        .liquidity_manager_address
        .ok_or_else(|| anyhow::anyhow!("liquidity_manager_address is None"))?;

    let provider = build_provider(&token.rpc_urls)
        .with_context(|| format!("failed to construct provider for '{}'", token.label))?;

    let liquidity_manager =
        LiquidityManagerContract::new(provider.clone(), lm_address).with_legacy_tx(token.legacy_tx);

    // Fetch underlying token address from LiquidityManager
    let underlying_address = liquidity_manager
        .underlying_token()
        .await
        .with_context(|| {
            format!(
                "failed to fetch underlying token address for '{}'",
                token.label
            )
        })?;

    let underlying_token =
        Erc20Contract::new(provider, underlying_address).with_legacy_tx(token.legacy_tx);

    Ok(ChainContext {
        label: token.label.clone(),
        chain_id: token.chain_id,
        liquidity_manager,
        underlying_token,
    })
}

fn build_provider(rpc_urls: &[String]) -> Result<NormalProvider> {
    if rpc_urls.is_empty() {
        bail!("provider requires at least one RPC URL");
    }
    if rpc_urls.len() == 1 {
        get_provider(
            rpc_urls
                .first()
                .expect("one url validated via length check"),
        )
    } else {
        get_provider_with_fallback(rpc_urls)
    }
}

fn load_tokens_config(path: &Path) -> Result<Vec<TokenEntry>> {
    if let Some(tokens) = load_tokens_config_from_env()? {
        return Ok(tokens);
    }
    let tokens_file = load_tokens_from_path(path)?;
    Ok(tokens_file.tokens)
}

fn load_tokens_config_from_env() -> Result<Option<Vec<TokenEntry>>> {
    match env::var("TOKENS_COMPRESSED") {
        Ok(value) => {
            if value.trim().is_empty() {
                bail!("TOKENS_COMPRESSED is set but empty");
            }
            let tokens_file = load_tokens_from_compressed(&value)
                .context("failed to parse TOKENS_COMPRESSED payload")?;
            Ok(Some(tokens_file.tokens))
        }
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => {
            bail!("TOKENS_COMPRESSED contains invalid unicode")
        }
    }
}

fn parse_private_key(input: &str) -> Result<B256> {
    let normalized = input.trim().strip_prefix("0x").unwrap_or(input.trim());
    let bytes = hex::decode(normalized)
        .with_context(|| format!("failed to decode private key hex: {input}"))?;
    if bytes.len() != 32 {
        bail!("private key must be 32 bytes, got {}", bytes.len());
    }
    Ok(B256::from_slice(&bytes))
}
