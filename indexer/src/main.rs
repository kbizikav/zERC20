use std::{env, path::PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use log::{info, warn};
use sqlx::{Executor, postgres::PgPoolOptions};
use tokio::task::JoinError;
use tree_indexer::{
    config::IndexerConfig,
    jobs::{EventSyncJobBuilder, RootProverJobBuilder, TreeIngestionJobBuilder},
    server,
};

#[derive(Parser, Debug)]
#[command(name = "tree-indexer", about = "Runs zERC20 indexing jobs")]
struct Cli {
    #[arg(
        long,
        env = "TOKENS_FILE_PATH",
        default_value = "../config/tokens.json"
    )]
    tokens: PathBuf,
    #[arg(long, default_value_t = 10)]
    max_connections: u32,
    /// Run each job once and exit instead of looping forever
    #[arg(long)]
    once: bool,
    #[arg(long, env = "LISTEN_ADDR", default_value = "127.0.0.1:8080")]
    listen_addr: String,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    env_logger::init();

    let cli = Cli::parse();
    let config = IndexerConfig::load(&cli.tokens)
        .with_context(|| format!("failed to load tokens from {}", cli.tokens.display()))?;

    let pool = PgPoolOptions::new()
        .max_connections(cli.max_connections)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                conn.execute("SET client_min_messages TO WARNING").await?;
                Ok(())
            })
        })
        .connect(&config.database_url)
        .await
        .context("failed to connect to postgres")?;

    let run_sync = env::var("IS_SYNC")
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let event_job = EventSyncJobBuilder::new(
        pool.clone(),
        config.event_indexer.clone(),
        config.tokens.clone(),
    )
    .into_job()
    .context("failed to construct event sync job")?;

    let tree_job =
        TreeIngestionJobBuilder::new(pool.clone(), config.tree.clone(), config.tokens.clone())
            .into_job()
            .context("failed to construct tree ingestion job")?;

    let tree_config = config
        .tree
        .build_tree_config()
        .context("failed to build merkle tree config for root prover job")?;

    let root_job = RootProverJobBuilder::new(
        pool.clone(),
        config.root.clone(),
        tree_config.clone(),
        config.tree.height,
        config.tokens.clone(),
    )
    .into_job()
    .context("failed to construct root prover job")?;

    if cli.once {
        if run_sync {
            event_job.run_once().await;
            tree_job.run_once().await;
            root_job.run_once().await?;
        } else {
            info!("IS_SYNC is not set to 'true'; skipping job execution in --once mode");
        }
        return Ok(());
    }

    let mut server_future = Box::pin(server::run_http_server(
        &cli.listen_addr,
        pool.clone(),
        &config.tokens,
        tree_config.clone(),
        config.tree.height,
    ));

    if run_sync {
        info!(
            "starting indexer jobs with HTTP server on {} (IS_SYNC=true)",
            cli.listen_addr
        );
        let event_handle = tokio::spawn(async move { event_job.run_forever().await });
        let tree_handle = tokio::spawn(async move { tree_job.run_forever().await });
        let root_handle = tokio::spawn(async move { root_job.run_forever().await });

        tokio::select! {
            res = &mut server_future => {
                res?;
            }
            res = event_handle => {
                handle_job_exit("event sync", res)?;
            }
            res = tree_handle => {
                handle_job_exit("tree ingestion", res)?;
            }
            res = root_handle => {
                handle_job_exit("root prover", res)?;
            }
        }
    } else {
        info!(
            "IS_SYNC is not 'true'; starting HTTP server on {} without background jobs",
            cli.listen_addr
        );
        server_future.await?;
    }

    Ok(())
}

fn handle_job_exit(name: &str, result: std::result::Result<Result<()>, JoinError>) -> Result<()> {
    match result {
        Ok(inner) => {
            inner.with_context(|| format!("{name} job exited with an error"))?;
        }
        Err(join_err) => return Err(join_err.into()),
    }
    warn!("{name} job terminated; shutting down");
    Ok(())
}
