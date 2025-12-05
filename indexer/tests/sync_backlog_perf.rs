mod common;

use std::{
    convert::TryFrom,
    path::Path,
    time::{Duration, Instant},
};

use alloy::{
    primitives::{Address, B256, U256},
    sol,
};
use anyhow::{Context, Result};
use client_common::{
    contracts::{
        utils::{get_address_from_private_key, get_provider},
        z_erc20::ZErc20Contract,
    },
    tokens::TokenEntry,
};
use common::{
    TestDatabase,
    anvil::{
        AnvilInstance, DEFAULT_ANVIL_CHAIN_ID, await_receipt, find_unused_port,
        is_binary_available, parse_private_key, wait_for_anvil,
    },
};
use futures::{StreamExt, stream::FuturesUnordered};
use sqlx::{PgPool, migrate::Migrator};
use tree_indexer::{
    config::{EventJobConfig, TreeJobConfig},
    jobs::{EventSyncJobBuilder, TreeIngestionJobBuilder},
};

const SYNC_BENCH_TRANSFER_COUNTS: [usize; 7] = [1, 10, 100, 1000, 10_000, 100_000, 1_000_000];

struct PerfResult {
    event_count: usize,
    seed_duration: Duration,
    event_duration: Duration,
    tree_duration: Duration,
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "expensive; run manually with cargo test -p tree-indexer sync_backlog_perf -- --ignored --nocapture"]
async fn sync_backlog_perf() -> Result<()> {
    env_logger::try_init().ok();

    let anvil_bin = std::env::var("ANVIL_BIN").unwrap_or_else(|_| "anvil".to_string());
    if !is_binary_available(&anvil_bin).await {
        eprintln!("skipping perf bench: anvil binary not found ({anvil_bin})");
        return Ok(());
    }

    let transfer_counts: [usize; 7] = [1, 10, 100, 1000, 10_000, 100_000, 1_000_000];

    println!(
        "running sync backlog perf bench for counts: {:?}",
        transfer_counts
    );

    for count in SYNC_BENCH_TRANSFER_COUNTS {
        println!("\n=== benchmarking backlog of {count} transfers ===");
        match run_single_case(&anvil_bin, count).await {
            Ok(result) => {
                let total = result.seed_duration + result.event_duration + result.tree_duration;
                println!(
                    "transfers requested: {}\ningested events: {}\nseed time: {:.2?}\nevent sync: {:.2?} ({:.1} ev/s)\ntree ingestion: {:.2?} ({:.1} ev/s)\ntotal: {:.2?}",
                    count,
                    result.event_count,
                    result.seed_duration,
                    result.event_duration,
                    throughput(result.event_count, result.event_duration),
                    result.tree_duration,
                    throughput(result.event_count, result.tree_duration),
                    total,
                );
            }
            Err(err) => {
                eprintln!("bench case for {count} transfers failed: {err:?}");
            }
        }
    }

    Ok(())
}

async fn run_single_case(anvil_bin: &str, transfer_count: usize) -> Result<PerfResult> {
    let port = find_unused_port().context("failed to allocate anvil port")?;
    let anvil = AnvilInstance::spawn(anvil_bin, port, DEFAULT_ANVIL_CHAIN_ID)
        .await
        .context("failed to spawn anvil")?;

    let rpc_url = anvil.rpc_url();
    let provider = get_provider(&rpc_url)?;
    wait_for_anvil(&provider).await?;

    let database = TestDatabase::create("sync_perf").await?;
    let migrator = Migrator::new(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/migrations"
    )))
    .await
    .context("failed to load migrations for perf bench")?;
    migrator
        .run(database.pool())
        .await
        .context("failed to run migrations for perf bench")?;

    let deployer_key =
        parse_private_key("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")?;
    let deployer_address = get_address_from_private_key(deployer_key);

    let contract = ZErc20Contract::deploy(
        provider.clone(),
        deployer_key,
        "TestToken".to_string(),
        "TT".to_string(),
        deployer_address,
        deploy_mock_endpoint(&provider, deployer_key).await?,
        18,
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

    let mint_amount = U256::from(
        (transfer_count as u64)
            .saturating_mul(2)
            .saturating_add(10_000),
    );
    await_receipt(
        contract
            .mint(deployer_key, deployer_address, mint_amount)
            .await
            .context("mint transaction failed to submit")?,
    )
    .await?;

    let token_entry = TokenEntry {
        label: format!("perf-{transfer_count}"),
        token_address: contract.address(),
        verifier_address: deployer_address,
        liquidity_manager_address: None,
        adaptor_address: None,
        eid: None,
        layerzero_endpoint: None,
        chain_id: DEFAULT_ANVIL_CHAIN_ID,
        deployed_block_number: 0,
        rpc_urls: vec![rpc_url.clone()],
        legacy_tx: false,
    };

    let event_job = EventSyncJobBuilder::new(
        database.pool().clone(),
        EventJobConfig::default(),
        vec![token_entry.clone()],
    )
    .into_job()
    .context("failed to construct event job")?;

    let tree_job = TreeIngestionJobBuilder::new(
        database.pool().clone(),
        TreeJobConfig::default(),
        vec![token_entry.clone()],
    )
    .into_job()
    .context("failed to construct tree job")?;

    // Initial sync to register the token row before seeding a backlog.
    event_job.run_once().await;
    tree_job.run_once().await;

    let token_id =
        fetch_token_id(database.pool(), contract.address(), token_entry.chain_id).await?;

    let seed_started = Instant::now();
    seed_transfers(&contract, deployer_key, transfer_count).await?;
    let seed_duration = seed_started.elapsed();

    let event_started = Instant::now();
    event_job.run_once().await;
    let event_duration = event_started.elapsed();

    let tree_started = Instant::now();
    tree_job.run_once().await;
    let tree_duration = tree_started.elapsed();

    let final_count = assert_tree_matches_events(database.pool(), token_id).await?;

    database.cleanup().await?;
    anvil.stop().await?;

    Ok(PerfResult {
        event_count: final_count
            .try_into()
            .context("event count exceeds usize during perf bench")?,
        seed_duration,
        event_duration,
        tree_duration,
    })
}

async fn seed_transfers(
    contract: &ZErc20Contract,
    deployer_key: alloy::primitives::B256,
    transfer_count: usize,
) -> Result<()> {
    const RECEIPT_BATCH: usize = 500;
    let amount = U256::from(1u64);
    let mut buf = [0u8; 20];
    // Set a non-zero prefix to avoid the ERC20 zero-address guard.
    buf[..12].fill(0xAA);

    let mut pending = Vec::with_capacity(RECEIPT_BATCH);
    for i in 0..transfer_count {
        let idx = i as u64;
        buf[12..20].copy_from_slice(&idx.to_be_bytes());
        let recipient = Address::from_slice(&buf);
        let tx = contract
            .transfer(deployer_key, recipient, amount)
            .await
            .context("transfer transaction failed to submit")?;
        pending.push(tx);

        if pending.len() >= RECEIPT_BATCH {
            await_receipts(pending.drain(..)).await?;
        }

        if (i + 1) % 10_000 == 0 {
            println!("seeded {} transfers", i + 1);
        }
    }

    if !pending.is_empty() {
        await_receipts(pending.drain(..)).await?;
    }

    Ok(())
}

async fn await_receipts<I>(iter: I) -> Result<()>
where
    I: IntoIterator<Item = alloy::providers::PendingTransactionBuilder<alloy::network::Ethereum>>,
{
    let mut tasks = FuturesUnordered::new();
    for tx in iter {
        tasks.push(async move { await_receipt(tx).await });
    }

    while let Some(result) = tasks.next().await {
        result?;
    }

    Ok(())
}

// Minimal endpoint mock; only setDelegate is required by zERC20 tests.
sol! {
    #[sol(rpc, bytecode = "0x6080604052348015600e575f5ffd5b50609b80601a5f395ff3fe6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063ca5eb5e114602a575b5f5ffd5b60386035366004603a565b50565b005b5f602082840312156049575f5ffd5b81356001600160a01b0381168114605e575f5ffd5b939250505056fea2646970667358221220203e0af40b06ba1a622ec2fd5c65d8e368b903da54e8b20e0e04e97d4d52d77b64736f6c634300081e0033", deployed_bytecode = "0x6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063ca5eb5e114602a575b5f5ffd5b60386035366004603a565b50565b005b5f602082840312156049575f5ffd5b81356001600160a01b0381168114605e575f5ffd5b939250505056fea2646970667358221220203e0af40b06ba1a622ec2fd5c65d8e368b903da54e8b20e0e04e97d4d52d77b64736f6c634300081e0033")]
    contract MockEndpoint {
        function setDelegate(address _delegate) external {}
    }
}

async fn deploy_mock_endpoint(
    provider: &client_common::contracts::utils::NormalProvider,
    deployer_key: B256,
) -> Result<Address> {
    let signer = client_common::contracts::utils::get_provider_with_signer(provider, deployer_key);
    let instance = MockEndpoint::deploy(signer)
        .await
        .context("failed to deploy mock endpoint contract")?;

    Ok(*instance.address())
}

fn throughput(events: usize, duration: Duration) -> f64 {
    if duration.is_zero() {
        return events as f64;
    }
    events as f64 / duration.as_secs_f64()
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
