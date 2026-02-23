mod alert;
mod balance;
mod config;
mod crosschain;
mod indexer_monitor;

use std::path::PathBuf;

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
        default_value = "config/watcher.yaml"
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
    let config = load_config(&cli.config)?;
    info!("loaded config from {}", cli.config.display());

    let mut alert_manager = AlertManager::new(
        config.discord_webhook_url.clone(),
        config.alert.cooldown_seconds,
    );

    let mut indexer_monitor = config
        .indexer
        .as_ref()
        .map(|cfg| IndexerMonitor::new(cfg.clone()));

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
        return Ok(());
    }

    info!(
        "starting watcher loop, interval={}s",
        config.interval_seconds
    );

    let mut ticker = time::interval(std::time::Duration::from_secs(config.interval_seconds));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

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
    if let Some(cc_config) = config.crosschain.as_ref() {
        info!("running crosschain checks...");
        let alerts = crosschain::check_crosschain(cc_config).await;
        all_alerts.extend(alerts);
    }

    all_alerts
}
