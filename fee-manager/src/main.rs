use std::{env, path::Path, time::Duration};

use alloy::{
    primitives::{Address, B256, U256, address},
    providers::Provider,
};
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

/// ERC-7528 native token address convention
const NATIVE_TOKEN: Address = address!("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");

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

/// Represents the underlying token type for a LiquidityManager.
enum UnderlyingToken {
    /// Native token (ETH) - use provider.get_balance()
    Native(NormalProvider),
    /// ERC20 token - use balanceOf()
    Erc20(Erc20Contract),
}

/// Per-chain context for fee management operations.
struct ChainContext {
    label: String,
    #[allow(dead_code)]
    chain_id: u64,
    liquidity_manager: LiquidityManagerContract,
    underlying: UnderlyingToken,
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
        let mut ticker = time::interval(self.interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        // The first tick() fires immediately, so the initial execution happens here
        loop {
            ticker.tick().await;
            if let Err(err) = self.execute_once().await {
                error!("fee parameter update failed: {err:?}");
            }
        }
    }

    async fn execute_once(&self) -> Result<()> {
        info!(
            "Fetching liquidity (balance - feeSurplus) across {} chains...",
            self.chains.len()
        );

        // Step 1: Fetch liquidity from all chains
        // Only proceed with update if all chains succeed
        let mut liquidities: Vec<(&ChainContext, U256)> = Vec::with_capacity(self.chains.len());
        let mut total_liquidity = U256::ZERO;
        let mut failed_chains: Vec<&str> = Vec::new();

        for chain in &self.chains {
            match self.fetch_liquidity(chain).await {
                Ok(liquidity) => {
                    info!(
                        "[{}] liquidity: {} (balance - feeSurplus)",
                        chain.label, liquidity
                    );
                    total_liquidity = total_liquidity.saturating_add(liquidity);
                    liquidities.push((chain, liquidity));
                }
                Err(err) => {
                    error!("[{}] failed to fetch liquidity: {err:?}", chain.label);
                    failed_chains.push(&chain.label);
                }
            }
        }

        // Skip update if any chain failed, as the target would be inaccurate
        if !failed_chains.is_empty() {
            warn!(
                "Skipping fee parameter update: failed to fetch liquidity for {} chain(s): {}",
                failed_chains.len(),
                failed_chains.join(", ")
            );
            return Ok(());
        }

        if liquidities.is_empty() {
            warn!("No chains configured; skipping fee parameter update");
            return Ok(());
        }

        // Step 2: Calculate target liquidity per chain
        let chain_count = U256::from(liquidities.len());
        let target_liquidity = total_liquidity / chain_count;
        let k = U256::from(self.k_bps);

        info!(
            "Total liquidity: {} across {} chains",
            total_liquidity,
            liquidities.len()
        );
        info!(
            "Target per chain: {} (k: {} bps)",
            target_liquidity, self.k_bps
        );

        // Step 3: Update fee params on each chain
        let mut updated_count = 0;
        let mut skipped_count = 0;
        for (chain, _liquidity) in liquidities {
            match self.update_fee_params(chain, target_liquidity, k).await {
                Ok(Some(tx_hash)) => {
                    info!("[{}] setFeeParams tx confirmed: {:?}", chain.label, tx_hash);
                    updated_count += 1;
                }
                Ok(None) => {
                    // Skipped due to no change
                    skipped_count += 1;
                }
                Err(err) => {
                    error!("[{}] failed to update fee params: {err:?}", chain.label);
                    // Continue with other chains
                }
            }
        }

        info!(
            "Fee parameter update complete: {} updated, {} skipped (unchanged). Next update in {} seconds.",
            updated_count,
            skipped_count,
            self.interval.as_secs()
        );
        Ok(())
    }

    /// Fetch effective liquidity for a chain.
    /// Liquidity is defined as `balance - feeSurplus` to match the on-chain incentive curve
    /// (see contracts/src/liquidity/LiquidityManager.sol lines 232-250).
    async fn fetch_liquidity(&self, chain: &ChainContext) -> Result<U256> {
        let lm_address = chain.liquidity_manager.address();

        // Fetch balance based on underlying token type
        let balance = match &chain.underlying {
            UnderlyingToken::Native(provider) => {
                provider.get_balance(lm_address).await.with_context(|| {
                    format!(
                        "failed to fetch native balance for {} at {:?}",
                        chain.label, lm_address
                    )
                })?
            }
            UnderlyingToken::Erc20(token) => {
                token.balance_of(lm_address).await.with_context(|| {
                    format!(
                        "failed to fetch ERC20 balance for {} at {:?}",
                        chain.label, lm_address
                    )
                })?
            }
        };

        let fee_surplus = chain
            .liquidity_manager
            .fee_surplus()
            .await
            .with_context(|| format!("failed to fetch feeSurplus for {}", chain.label))?;

        // liquidity = balance - feeSurplus
        let liquidity = balance.saturating_sub(fee_surplus);
        Ok(liquidity)
    }

    async fn update_fee_params(
        &self,
        chain: &ChainContext,
        target_liquidity: U256,
        k: U256,
    ) -> Result<Option<B256>> {
        // Fetch current feeParams and check for changes
        let (current_target, current_k) = chain
            .liquidity_manager
            .fee_params()
            .await
            .with_context(|| format!("failed to fetch current feeParams for {}", chain.label))?;

        if current_target == target_liquidity && current_k == k {
            info!(
                "[{}] feeParams unchanged (target: {}, k: {}), skipping",
                chain.label, target_liquidity, k
            );
            return Ok(None);
        }

        info!(
            "[{}] Updating feeParams (target: {} -> {}, k: {} -> {})",
            chain.label, current_target, target_liquidity, current_k, k
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

        Ok(Some(receipt.transaction_hash))
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

    // Solidity requires k <= 10_000 (see contracts/src/libraries/IncentiveLib.sol:186)
    if cli.k_bps > 10_000 {
        bail!(
            "FEE_MANAGER_K_BPS must be <= 10000 (100%), got {}",
            cli.k_bps
        );
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

    // Build chain contexts - fail fast if any chain is unreachable
    // This ensures targetLiquidity is always computed across all configured chains.
    // If a chain's RPC is down at startup, the operator must fix it before restarting.
    let mut chains = Vec::with_capacity(tokens_with_lm.len());
    for token in &tokens_with_lm {
        let (ctx, underlying_address) = build_chain_context(token)
            .await
            .with_context(|| format!("failed to build chain context for '{}'", token.label))?;

        let underlying_type = if underlying_address == NATIVE_TOKEN {
            "Native (ETH)"
        } else {
            "ERC20"
        };
        info!(
            "[{}] LiquidityManager: {:?}, Underlying: {:?} ({})",
            ctx.label,
            ctx.liquidity_manager.address(),
            underlying_address,
            underlying_type
        );
        chains.push(ctx);
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

async fn build_chain_context(token: &TokenEntry) -> Result<(ChainContext, Address)> {
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

    // Check if underlying is native token (ETH) or ERC20
    let underlying = if underlying_address == NATIVE_TOKEN {
        UnderlyingToken::Native(provider)
    } else {
        UnderlyingToken::Erc20(
            Erc20Contract::new(provider, underlying_address).with_legacy_tx(token.legacy_tx),
        )
    };

    Ok((
        ChainContext {
            label: token.label.clone(),
            chain_id: token.chain_id,
            liquidity_manager,
            underlying,
        },
        underlying_address,
    ))
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
    let bytes =
        hex::decode(normalized).context("failed to decode private key: invalid hex format")?;
    if bytes.len() != 32 {
        bail!("private key must be 32 bytes, got {}", bytes.len());
    }
    Ok(B256::from_slice(&bytes))
}
