mod common;

use std::{convert::TryFrom, path::Path};

use alloy::{
    primitives::{Address, B256, U256},
    providers::Provider,
    sol,
};
use anyhow::{Context, Result};
use client_common::{
    contracts::{
        utils::{NormalProvider, get_address_from_private_key, get_provider},
        z_erc20::ZErc20Contract,
    },
    tokens::TokenMetadata,
};
use common::{
    TestDatabase,
    anvil::{
        AnvilInstance, DEFAULT_ANVIL_CHAIN_ID, await_receipt, find_unused_port,
        is_binary_available, parse_private_key, wait_for_anvil,
    },
};
use sqlx::migrate::Migrator;
use zerc20_tree_indexer::events::{
    BLOCK_SPAN_RECOMMENDED, EventIndexer, EventIndexerConfig, FORWARD_SCAN_OVERLAP_RECOMMENDED,
    REORG_CHECK_WINDOW_RECOMMENDED,
};

// ---------------------------------------------------------------------------
// Anvil snapshot / revert helpers
// ---------------------------------------------------------------------------

/// Takes an EVM snapshot and returns a snapshot id.
async fn evm_snapshot(provider: &NormalProvider) -> Result<U256> {
    let result: U256 = provider
        .client()
        .request_noparams("evm_snapshot")
        .await
        .context("evm_snapshot RPC failed")?;
    Ok(result)
}

/// Reverts the chain to a previous snapshot. All blocks after the snapshot are removed.
async fn evm_revert(provider: &NormalProvider, snapshot_id: U256) -> Result<bool> {
    let result: bool = provider
        .client()
        .request("evm_revert", (snapshot_id,))
        .await
        .context("evm_revert RPC failed")?;
    Ok(result)
}

// ---------------------------------------------------------------------------
// Shared setup
// ---------------------------------------------------------------------------

struct TestHarness {
    anvil: AnvilInstance,
    database: TestDatabase,
    provider: NormalProvider,
    contract: ZErc20Contract,
    deployer_key: B256,
    deployer_address: Address,
    metadata: TokenMetadata,
}

impl TestHarness {
    async fn setup(test_name: &str) -> Result<Option<Self>> {
        let anvil_bin = std::env::var("ANVIL_BIN").unwrap_or_else(|_| "anvil".to_string());
        if !is_binary_available(&anvil_bin).await {
            eprintln!("skipping test: anvil binary not found ({anvil_bin})");
            return Ok(None);
        }

        let port = match find_unused_port() {
            Ok(port) => port,
            Err(err) => {
                eprintln!("skipping test: failed to allocate free TCP port ({err:?})");
                return Ok(None);
            }
        };
        let anvil = AnvilInstance::spawn(&anvil_bin, port, DEFAULT_ANVIL_CHAIN_ID).await?;

        let rpc_url = anvil.rpc_url();
        let provider = get_provider(&rpc_url)?;
        wait_for_anvil(&provider).await?;

        let database = match TestDatabase::create(test_name).await {
            Ok(db) => db,
            Err(err) => {
                eprintln!("skipping test: failed to start postgres container ({err:?})");
                return Ok(None);
            }
        };
        let migrator = Migrator::new(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/migrations"
        )))
        .await
        .context("failed to load migrations")?;
        migrator
            .run(database.pool())
            .await
            .context("failed to run migrations")?;

        let deployer_key = parse_private_key(
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        )?;
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
                .context("set_minter failed")?,
        )
        .await?;

        let metadata = TokenMetadata {
            token_address: contract.address(),
            verifier_address: deployer_address,
            chain_id: DEFAULT_ANVIL_CHAIN_ID,
        };

        Ok(Some(Self {
            anvil,
            database,
            provider,
            contract,
            deployer_key,
            deployer_address,
            metadata,
        }))
    }

    fn make_indexer_config(&self) -> Result<EventIndexerConfig> {
        Ok(EventIndexerConfig::new(
            BLOCK_SPAN_RECOMMENDED,
            FORWARD_SCAN_OVERLAP_RECOMMENDED,
            REORG_CHECK_WINDOW_RECOMMENDED,
        )?)
    }

    async fn make_indexer(&self) -> Result<EventIndexer> {
        let config = self.make_indexer_config()?;
        Ok(EventIndexer::new(
            self.contract.clone(),
            self.database.pool().clone(),
            0,
            self.metadata,
            config,
            "test-token",
        )
        .await?)
    }

    fn pool(&self) -> &sqlx::PgPool {
        self.database.pool()
    }

    async fn token_id(&self) -> Result<i64> {
        let id: i64 =
            sqlx::query_scalar("SELECT id FROM tokens WHERE token_address = $1 AND chain_id = $2")
                .bind(self.contract.address().as_slice())
                .bind(i64::try_from(self.metadata.chain_id)?)
                .fetch_one(self.pool())
                .await
                .context("token metadata row missing")?;
        Ok(id)
    }

    async fn event_indices(&self) -> Result<Vec<i64>> {
        let token_id = self.token_id().await?;
        let rows = sqlx::query_as::<_, (i64,)>(
            "SELECT event_index FROM indexed_transfer_events WHERE token_id = $1 ORDER BY event_index",
        )
        .bind(token_id)
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn contiguous_index(&self) -> Result<i64> {
        let token_id = self.token_id().await?;
        let val: i64 = sqlx::query_scalar(
            "SELECT contiguous_index FROM event_indexer_state WHERE token_id = $1",
        )
        .bind(token_id)
        .fetch_one(self.pool())
        .await?;
        Ok(val)
    }

    async fn indexed_block_count(&self) -> Result<i64> {
        let token_id = self.token_id().await?;
        let val: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM indexed_blocks WHERE token_id = $1")
                .bind(token_id)
                .fetch_one(self.pool())
                .await?;
        Ok(val)
    }

    async fn cleanup(self) -> Result<()> {
        self.database.cleanup().await?;
        self.anvil.stop().await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Mock endpoint (same as event_indexer.rs)
// ---------------------------------------------------------------------------

sol! {
    #[sol(rpc, bytecode = "0x6080604052348015600e575f5ffd5b50609b80601a5f395ff3fe6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063ca5eb5e114602a575b5f5ffd5b60386035366004603a565b50565b005b5f602082840312156049575f5ffd5b81356001600160a01b0381168114605e575f5ffd5b939250505056fea2646970667358221220203e0af40b06ba1a622ec2fd5c65d8e368b903da54e8b20e0e04e97d4d52d77b64736f6c634300081e0033", deployed_bytecode = "0x6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063ca5eb5e114602a575b5f5ffd5b60386035366004603a565b50565b005b5f602082840312156049575f5ffd5b81356001600160a01b0381168114605e575f5ffd5b939250505056fea2646970667358221220203e0af40b06ba1a622ec2fd5c65d8e368b903da54e8b20e0e04e97d4d52d77b64736f6c634300081e0033")]
    contract MockEndpoint {
        function setDelegate(address _delegate) external {}
    }
}

async fn deploy_mock_endpoint(provider: &NormalProvider, deployer_key: B256) -> Result<Address> {
    let signer = client_common::contracts::utils::get_provider_with_signer(provider, deployer_key);
    let instance = MockEndpoint::deploy(signer)
        .await
        .context("failed to deploy mock endpoint contract")?;
    Ok(*instance.address())
}

// ===========================================================================
// Tests
// ===========================================================================

/// Test that the indexer detects a reorg (block hash mismatch) and rolls back
/// the affected events, then re-syncs the new canonical events.
#[tokio::test(flavor = "multi_thread")]
async fn reorg_detection_and_rollback() -> Result<()> {
    let Some(h) = TestHarness::setup("reorg_test").await? else {
        return Ok(());
    };

    let recipient_a = Address::from_slice(&[0xAA; 20]);
    let recipient_b = Address::from_slice(&[0xBB; 20]);

    // -- Phase 1: Mint + transfer to create initial events (indices 0, 1) ----
    await_receipt(
        h.contract
            .mint(h.deployer_key, h.deployer_address, U256::from(10_000u64))
            .await?,
    )
    .await?;
    await_receipt(
        h.contract
            .transfer(h.deployer_key, recipient_a, U256::from(100u64))
            .await?,
    )
    .await?;

    let indexer = h.make_indexer().await?;
    indexer.sync().await?;

    assert_eq!(h.event_indices().await?, vec![0, 1]);
    assert_eq!(h.contiguous_index().await?, 1);
    assert!(
        h.indexed_block_count().await? > 0,
        "blocks should be recorded"
    );

    // -- Phase 2: Snapshot, add events, sync, then revert --------------------
    let snapshot_id = evm_snapshot(&h.provider).await?;

    // These events will be on the "old" fork.
    await_receipt(
        h.contract
            .transfer(h.deployer_key, recipient_b, U256::from(200u64))
            .await?,
    )
    .await?;
    await_receipt(
        h.contract
            .transfer(h.deployer_key, recipient_b, U256::from(300u64))
            .await?,
    )
    .await?;

    indexer.sync().await?;
    assert_eq!(
        h.event_indices().await?,
        vec![0, 1, 2, 3],
        "pre-reorg sync should capture 4 events"
    );

    // Revert to the snapshot — this removes the blocks that contained events 2 and 3.
    let reverted = evm_revert(&h.provider, snapshot_id).await?;
    assert!(reverted, "evm_revert should succeed");

    // -- Phase 3: Create *different* events on the new canonical chain -------
    // After revert, the contract state is back to index=2 (0 and 1 exist).
    // New transfers will re-use indices 2, 3 but in new blocks with new hashes.
    await_receipt(
        h.contract
            .transfer(h.deployer_key, recipient_a, U256::from(50u64))
            .await?,
    )
    .await?;

    // -- Phase 4: Sync — should detect reorg and rollback --------------------
    indexer.sync().await?;

    let indices = h.event_indices().await?;
    assert!(
        indices.contains(&0) && indices.contains(&1) && indices.contains(&2),
        "after reorg recovery, events 0, 1, 2 should exist; got {indices:?}"
    );
    // Event 3 (from old fork) should have been removed. New fork only produced
    // one transfer, so we expect exactly 3 events now.
    assert_eq!(
        indices.len(),
        3,
        "should have exactly 3 events after reorg; got {indices:?}"
    );

    let ci = h.contiguous_index().await?;
    assert!(ci >= 2, "contiguous_index should be at least 2; got {ci}");

    h.cleanup().await?;
    Ok(())
}

/// Test that indexed_blocks rows are pruned when they fall outside the
/// reorg_check_window.
#[tokio::test(flavor = "multi_thread")]
async fn old_blocks_are_pruned() -> Result<()> {
    let Some(h) = TestHarness::setup("prune_test").await? else {
        return Ok(());
    };

    // Use a very small reorg_check_window so pruning kicks in quickly.
    let config = EventIndexerConfig::new(
        BLOCK_SPAN_RECOMMENDED,
        FORWARD_SCAN_OVERLAP_RECOMMENDED,
        2, // tiny window
    )?;

    let indexer = EventIndexer::new(
        h.contract.clone(),
        h.pool().clone(),
        0,
        h.metadata,
        config,
        "test-token",
    )
    .await?;

    // Create several events across multiple blocks.
    await_receipt(
        h.contract
            .mint(h.deployer_key, h.deployer_address, U256::from(10_000u64))
            .await?,
    )
    .await?;

    for _ in 0..5 {
        await_receipt(
            h.contract
                .transfer(
                    h.deployer_key,
                    Address::from_slice(&[0xCC; 20]),
                    U256::from(1u64),
                )
                .await?,
        )
        .await?;
    }

    indexer.sync().await?;

    let block_count = h.indexed_block_count().await?;
    // With a window of 2, most old blocks should have been pruned.
    // The exact count depends on how many distinct blocks the events span,
    // but it should be small (at most window-sized).
    assert!(
        block_count <= 3,
        "expected at most 3 block records with window=2; got {block_count}"
    );

    h.cleanup().await?;
    Ok(())
}

/// Test that detect_reorg returns None when no reorg has occurred.
#[tokio::test(flavor = "multi_thread")]
async fn no_false_positive_reorg() -> Result<()> {
    let Some(h) = TestHarness::setup("no_reorg_test").await? else {
        return Ok(());
    };

    let indexer = h.make_indexer().await?;

    // Mint and sync.
    await_receipt(
        h.contract
            .mint(h.deployer_key, h.deployer_address, U256::from(1_000u64))
            .await?,
    )
    .await?;
    indexer.sync().await?;

    assert_eq!(h.event_indices().await?, vec![0]);

    // Add more events and sync again — no reorg should be detected.
    await_receipt(
        h.contract
            .transfer(
                h.deployer_key,
                Address::from_slice(&[0xDD; 20]),
                U256::from(10u64),
            )
            .await?,
    )
    .await?;
    indexer.sync().await?;

    assert_eq!(h.event_indices().await?, vec![0, 1]);
    assert_eq!(h.contiguous_index().await?, 1);

    // Third sync with no new events — still no reorg.
    indexer.sync().await?;
    assert_eq!(h.event_indices().await?, vec![0, 1]);

    h.cleanup().await?;
    Ok(())
}

/// Test that reorg_check_window=0 disables reorg detection entirely.
#[tokio::test(flavor = "multi_thread")]
async fn reorg_detection_disabled_when_window_zero() -> Result<()> {
    let Some(h) = TestHarness::setup("reorg_disabled_test").await? else {
        return Ok(());
    };

    let config = EventIndexerConfig::new(
        BLOCK_SPAN_RECOMMENDED,
        FORWARD_SCAN_OVERLAP_RECOMMENDED,
        0, // disabled
    )?;

    let indexer = EventIndexer::new(
        h.contract.clone(),
        h.pool().clone(),
        0,
        h.metadata,
        config,
        "test-token",
    )
    .await?;

    // Mint, sync, snapshot, add event, sync, revert, sync.
    await_receipt(
        h.contract
            .mint(h.deployer_key, h.deployer_address, U256::from(10_000u64))
            .await?,
    )
    .await?;
    indexer.sync().await?;
    assert_eq!(h.event_indices().await?, vec![0]);

    let snapshot_id = evm_snapshot(&h.provider).await?;

    await_receipt(
        h.contract
            .transfer(
                h.deployer_key,
                Address::from_slice(&[0xEE; 20]),
                U256::from(1u64),
            )
            .await?,
    )
    .await?;
    indexer.sync().await?;
    assert_eq!(h.event_indices().await?, vec![0, 1]);

    // Revert
    evm_revert(&h.provider, snapshot_id).await?;

    // With detection disabled, the old events remain (no rollback).
    // The sync may add the new event from the reverted chain on top.
    indexer.sync().await?;

    let indices = h.event_indices().await?;
    // The old event[1] should still be present since no rollback happened.
    assert!(
        indices.contains(&0) && indices.contains(&1),
        "with reorg detection disabled, old events should remain; got {indices:?}"
    );

    h.cleanup().await?;
    Ok(())
}
