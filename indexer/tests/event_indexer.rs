mod common;

use std::{convert::TryFrom, path::Path};

use alloy::primitives::{Address, U256};
use anyhow::{Context, Result};
use client_common::{
    contracts::{
        utils::{get_address_from_private_key, get_provider},
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
use tree_indexer::events::{
    BLOCK_SPAN_RECOMMENDED, EventIndexer, EventIndexerConfig, FORWARD_SCAN_OVERLAP_RECOMMENDED,
};

#[tokio::test(flavor = "multi_thread")]
async fn event_indexer_syncs_against_anvil() -> Result<()> {
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
    let anvil = AnvilInstance::spawn(&anvil_bin, port, DEFAULT_ANVIL_CHAIN_ID).await?;

    let rpc_url = anvil.rpc_url();
    let provider = get_provider(&rpc_url)?;
    wait_for_anvil(&provider).await?;

    let database = match TestDatabase::create("idx_test").await {
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
    .context("failed to load embedded migrations")?;
    migrator
        .run(database.pool())
        .await
        .context("failed to run migrations for test database")?;

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

    let recipient_a = Address::from_slice(&[0xAA; 20]);
    let recipient_b = Address::from_slice(&[0xBB; 20]);

    await_receipt(
        contract
            .transfer(deployer_key, recipient_a, U256::from(250u64))
            .await
            .context("transfer (recipient_a) failed to submit")?,
    )
    .await?;

    await_receipt(
        contract
            .transfer(deployer_key, recipient_b, U256::from(125u64))
            .await
            .context("transfer (recipient_b) failed to submit")?,
    )
    .await?;

    let metadata = TokenMetadata {
        token_address: contract.address(),
        verifier_address: deployer_address,
        chain_id: DEFAULT_ANVIL_CHAIN_ID,
    };

    let indexer_config =
        EventIndexerConfig::new(BLOCK_SPAN_RECOMMENDED, FORWARD_SCAN_OVERLAP_RECOMMENDED)?;

    let indexer = EventIndexer::new(
        contract.clone(),
        database.pool().clone(),
        0,
        metadata.clone(),
        indexer_config,
    )
    .await?;

    indexer.sync().await?;

    let token_id: i64 = sqlx::query_scalar(
        r#"
        SELECT id
        FROM tokens
        WHERE token_address = $1 AND chain_id = $2
        "#,
    )
    .bind(contract.address().as_slice())
    .bind(i64::try_from(metadata.chain_id)?)
    .fetch_one(database.pool())
    .await
    .context("token metadata row missing")?;

    let mut events = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT event_index, eth_block_number
        FROM indexed_transfer_events
        WHERE token_id = $1
        ORDER BY event_index
        "#,
    )
    .bind(token_id)
    .fetch_all(database.pool())
    .await?;

    assert_eq!(
        events.iter().map(|row| row.0).collect::<Vec<_>>(),
        vec![0, 1, 2],
        "initial sync should capture three sequential events"
    );

    let state = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT contiguous_index, last_synced_block
        FROM event_indexer_state
        WHERE token_id = $1
        "#,
    )
    .bind(token_id)
    .fetch_one(database.pool())
    .await?;
    assert_eq!(state.0, 2, "contiguous index should track last event");

    await_receipt(
        contract
            .mint(deployer_key, recipient_a, U256::from(50u64))
            .await
            .context("mint (round two) failed to submit")?,
    )
    .await?;

    indexer.sync().await?;

    events = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT event_index, eth_block_number
        FROM indexed_transfer_events
        WHERE token_id = $1
        ORDER BY event_index
        "#,
    )
    .bind(token_id)
    .fetch_all(database.pool())
    .await?;

    assert_eq!(
        events.iter().map(|row| row.0).collect::<Vec<_>>(),
        vec![0, 1, 2, 3],
        "second sync should append the new mint event"
    );

    database.cleanup().await?;
    anvil.stop().await?;

    Ok(())
}
