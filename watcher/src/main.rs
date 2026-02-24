mod alert;
mod balance;
mod config;
mod crosschain;
mod indexer_monitor;
mod stats;

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use log::{error, info};
use tokio::time::{self, MissedTickBehavior};

use alert::{Alert, AlertManager};
use config::load_config;
use indexer_monitor::IndexerMonitor;

#[derive(Parser, Debug)]
#[command(
    name = "zerc20-watcher",
    about = "Monitors zERC20 system health and sends Discord alerts"
)]
struct Cli {
    /// Path to the watcher YAML config file.
    #[arg(
        long,
        env = "WATCHER_CONFIG_PATH",
        value_name = "PATH",
        default_value = "watcher/watcher.yaml"
    )]
    config: PathBuf,

    /// Run checks once and exit instead of looping.
    #[arg(long)]
    once: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    env_logger::init();

    let cli = Cli::parse();

    // Also load .env from the config file's directory (e.g. watcher/.env)
    if let Some(config_dir) = cli.config.parent() {
        dotenvy::from_path(config_dir.join(".env")).ok();
    }

    let config = load_config(&cli.config)?;
    info!("loaded config from {}", cli.config.display());

    let mut alert_manager = AlertManager::new(
        config.discord_webhook_url.clone(),
        config.alert.cooldown_seconds,
    );

    let mut indexer_monitor = config
        .indexer
        .as_ref()
        .map(|cfg| IndexerMonitor::new(cfg.clone(), config.tokens.clone()));

    let stats_interval = config
        .stats_interval_seconds
        .filter(|&s| s > 0)
        .map(std::time::Duration::from_secs);

    if cli.once {
        let alerts = run_checks(&config, &mut indexer_monitor).await;
        if alerts.is_empty() {
            info!("all checks passed, no alerts");
        } else {
            info!("generated {} alert(s)", alerts.len());
            if let Err(err) = alert_manager.send_alerts(alerts).await {
                error!("failed to send alerts: {:?}", err);
            }
        }

        if stats_interval.is_some() {
            info!("collecting stats report...");
            let embeds = stats::collect_stats(&config).await;
            if let Err(err) = alert_manager.send_embeds(embeds).await {
                error!("failed to send stats: {:?}", err);
            }
        }

        return Ok(());
    }

    info!(
        "starting watcher loop, interval={}s",
        config.interval_seconds
    );

    let mut ticker = time::interval(std::time::Duration::from_secs(config.interval_seconds));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    // Track last stats send time; start far enough in the past to trigger on first eligible tick
    let mut last_stats_sent = stats_interval.map(|dur| Instant::now() - dur);

    loop {
        ticker.tick().await;

        let alerts = run_checks(&config, &mut indexer_monitor).await;
        if alerts.is_empty() {
            info!("check cycle complete, no alerts");
        } else {
            info!("check cycle complete, {} alert(s)", alerts.len());
            if let Err(err) = alert_manager.send_alerts(alerts).await {
                error!("failed to send alerts: {:?}", err);
            }
        }

        // Stats report
        if let (Some(interval), Some(last)) = (stats_interval, &mut last_stats_sent)
            && last.elapsed() >= interval
        {
            info!("collecting stats report...");
            let embeds = stats::collect_stats(&config).await;
            if let Err(err) = alert_manager.send_embeds(embeds).await {
                error!("failed to send stats: {:?}", err);
            }
            *last = Instant::now();
        }
    }
}

async fn run_checks(
    config: &config::WatcherConfig,
    indexer_monitor: &mut Option<IndexerMonitor>,
) -> Vec<Alert> {
    let mut all_alerts = Vec::new();

    // Domain 1: Balance checks
    if !config.accounts.is_empty() {
        info!("running balance checks...");
        let alerts = balance::check_balances(&config.accounts, &config.chains).await;
        all_alerts.extend(alerts);
    }

    // Domain 2: Indexer checks
    if let Some(monitor) = indexer_monitor.as_mut() {
        info!("running indexer checks...");
        let alerts = monitor.check().await;
        all_alerts.extend(alerts);
    }

    // Domain 3: Crosschain checks
    let has_crosschain = config
        .tokens
        .iter()
        .any(|t| t.crosschain_config_path.is_some());
    if has_crosschain {
        info!("running crosschain checks...");
        let cc = config.crosschain.clone().unwrap_or_default();
        let alerts = crosschain::check_crosschain(&config.tokens, &cc).await;
        all_alerts.extend(alerts);
    }

    all_alerts
}
