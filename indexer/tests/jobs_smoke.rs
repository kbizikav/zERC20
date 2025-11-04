mod common;

use std::{
    convert::TryFrom,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use alloy::primitives::{Address, U256};
use anyhow::{Context, Result};
use api_types::prover::CircuitKind;
use async_trait::async_trait;
use client_common::{
    contracts::{
        utils::{get_address_from_private_key, get_provider},
        z_erc20::ZErc20Contract,
    },
    prover::{DeciderClient, DeciderResult},
    tokens::TokenEntry,
};
use common::{
    TestDatabase,
    anvil::{
        AnvilInstance, DEFAULT_ANVIL_CHAIN_ID, await_receipt, find_unused_port,
        is_binary_available, parse_private_key, wait_for_anvil,
    },
};
use reqwest::Url;
use sqlx::{PgPool, migrate::Migrator};
use tree_indexer::{
    config::{EventJobConfig, RootJobConfig, TreeJobConfig},
    jobs::{EventSyncJobBuilder, RootProverJobBuilder, TreeIngestionJobBuilder},
    trees::HISTORY_WINDOW_RECOMMENDED,
};
struct MockDeciderClient {
    calls: AtomicUsize,
}

impl MockDeciderClient {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl DeciderClient for MockDeciderClient {
    async fn produce_decider_proof(
        &self,
        circuit: CircuitKind,
        ivc_proof: &[u8],
    ) -> DeciderResult<Vec<u8>> {
        println!("circuit={circuit}, _ivc_proof len: {}", ivc_proof.len());
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(vec![0xde, 0xad, 0xbe, 0xef])
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn event_and_tree_jobs_ingest_transfers() -> Result<()> {
    env_logger::try_init().ok();

    let anvil_bin = std::env::var("ANVIL_BIN").unwrap_or_else(|_| "anvil".to_string());
    if !is_binary_available(&anvil_bin).await {
        eprintln!("skipping test: anvil binary not found ({anvil_bin})");
        return Ok(());
    }

    let port = match find_unused_port() {
        Ok(port) => port,
        Err(err) => {
            eprintln!("skipping test: failed to allocate free TCP port for anvil ({err:?})");
            return Ok(());
        }
    };
    let anvil = match AnvilInstance::spawn(&anvil_bin, port, DEFAULT_ANVIL_CHAIN_ID).await {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping test: failed to start anvil instance ({err:?})");
            return Ok(());
        }
    };

    let rpc_url = anvil.rpc_url();
    let provider = get_provider(&rpc_url)?;
    wait_for_anvil(&provider).await?;

    let database = match TestDatabase::create("jobs_smoke").await {
        Ok(db) => db,
        Err(err) => {
            eprintln!("skipping test: failed to start postgres container ({err:?})");
            return Ok(());
        }
    };

    let migrator = Migrator::new(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/migrations"
    )))
    .await
    .context("failed to load migrations for jobs smoke test")?;
    migrator
        .run(database.pool())
        .await
        .context("failed to run migrations for jobs smoke test")?;

    let deployer_key =
        parse_private_key("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")?;
    let deployer_address = get_address_from_private_key(deployer_key);

    let contract = ZErc20Contract::deploy(
        provider.clone(),
        deployer_key,
        "TestToken".to_string(),
        "TT".to_string(),
        deployer_address,
    )
    .await
    .context("failed to deploy zERC20 contract")?;

    await_receipt(
        contract
            .set_minter(deployer_key, deployer_address)
            .await
            .context("set_minter transaction failed to submit")?,
    )
    .await?;

    await_receipt(
        contract
            .mint(deployer_key, deployer_address, U256::from(1_000u64))
            .await
            .context("mint transaction failed to submit")?,
    )
    .await?;

    let token_entry = TokenEntry {
        label: "anvil-test".to_string(),
        token_address: contract.address(),
        verifier_address: deployer_address,
        minter_address: None,
        chain_id: DEFAULT_ANVIL_CHAIN_ID,
        deployed_block_number: 0,
        rpc_urls: vec![rpc_url.clone()],
        legacy_tx: false,
    };

    let tree_job_config = TreeJobConfig::default();

    let event_job = EventSyncJobBuilder::new(
        database.pool().clone(),
        EventJobConfig::default(),
        vec![token_entry.clone()],
    )
    .into_job()
    .context("failed to construct event job")?;

    let tree_job = TreeIngestionJobBuilder::new(
        database.pool().clone(),
        tree_job_config.clone(),
        vec![token_entry.clone()],
    )
    .into_job()
    .context("failed to construct tree job")?;

    let artifacts_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate directory should have parent")
        .join("nova_artifacts");
    let root_job_config = RootJobConfig {
        interval_ms: 1_000,
        submit_interval_ms: 1_000,
        history_window: HISTORY_WINDOW_RECOMMENDED,
        prover_timeout: Duration::from_secs(5),
        prover_poll_interval: Duration::from_millis(50),
        prover_url: Url::parse("http://127.0.0.1:8080").expect("hardcoded prover url should parse"),
        submitter_private_key: deployer_key,
        artifacts_dir,
    };
    let tree_db_config = tree_job_config
        .build_tree_config()
        .context("failed to build tree config for root job")?;
    let mock_prover = Arc::new(MockDeciderClient::new());
    let root_job = RootProverJobBuilder::new(
        database.pool().clone(),
        root_job_config,
        tree_db_config,
        tree_job_config.height,
        vec![token_entry.clone()],
    )
    .with_prover(mock_prover.clone())
    .with_submission_enabled(false)
    .into_job()
    .context("failed to construct root job")?;

    event_job.run_once().await;
    tree_job.run_once().await;
    root_job
        .run_once()
        .await
        .context("root job run failed after initial sync")?;

    let token_id =
        fetch_token_id(database.pool(), contract.address(), token_entry.chain_id).await?;
    let mut last_event_count = assert_tree_matches_events(database.pool(), token_id).await?;

    let transfers = [
        (Address::from_slice(&[0xAA; 20]), U256::from(250u64)),
        (Address::from_slice(&[0xBB; 20]), U256::from(125u64)),
        (Address::from_slice(&[0xCC; 20]), U256::from(75u64)),
    ];

    for (recipient, amount) in transfers {
        await_receipt(
            contract
                .transfer(deployer_key, recipient, amount)
                .await
                .context("transfer transaction failed to submit")?,
        )
        .await?;

        event_job.run_once().await;
        tree_job.run_once().await;
        root_job
            .run_once()
            .await
            .context("root job run failed after transfer ingestion")?;

        let current = assert_tree_matches_events(database.pool(), token_id).await?;
        assert!(
            current >= last_event_count,
            "event count should not decrease (prev {last_event_count}, now {current})"
        );
        if current > last_event_count {
            last_event_count = current;
        } else {
            panic!("expected new transfer to advance tree index");
        }
    }

    event_job.run_once().await;
    tree_job.run_once().await;
    root_job
        .run_once()
        .await
        .context("root job run failed during idempotent check")?;
    let final_count = assert_tree_matches_events(database.pool(), token_id).await?;
    assert_eq!(
        final_count, last_event_count,
        "idempotent run should not change processed counts"
    );

    assert!(
        mock_prover.calls() > 0,
        "mock prover should be exercised by the root job"
    );

    database.cleanup().await?;
    anvil.stop().await?;

    Ok(())
}

async fn fetch_token_id(pool: &PgPool, token_address: Address, chain_id: u64) -> Result<i64> {
    let chain_id_i64 =
        i64::try_from(chain_id).context("chain id exceeds i64 range for token lookup")?;
    sqlx::query_scalar(
        r#"
        SELECT id
        FROM tokens
        WHERE token_address = $1 AND chain_id = $2
        "#,
    )
    .bind(token_address.as_slice())
    .bind(chain_id_i64)
    .fetch_one(pool)
    .await
    .context("token metadata row missing after initial sync")
}

async fn assert_tree_matches_events(pool: &PgPool, token_id: i64) -> Result<i64> {
    let event_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)::bigint
        FROM indexed_transfer_events
        WHERE token_id = $1
        "#,
    )
    .bind(token_id)
    .fetch_one(pool)
    .await
    .context("failed to count indexed transfer events")?;

    let latest_tree: i64 = sqlx::query_scalar(
        r#"
        SELECT COALESCE(MAX(tree_index), 0)
        FROM merkle_snapshots
        WHERE token_id = $1
        "#,
    )
    .bind(token_id)
    .fetch_one(pool)
    .await
    .context("failed to query latest tree index")?;

    assert_eq!(
        latest_tree, event_count,
        "tree index ({latest_tree}) should align with event count ({event_count})"
    );

    Ok(event_count)
}
