mod common;

use std::path::Path;

use alloy::primitives::{Address, U256};
use anyhow::{Context, Result};
use common::TestDatabase;
use sqlx::migrate::Migrator;
use tree_indexer::trees::{
    DbIncrementalMerkleTree, DbMerkleTreeConfig, HISTORY_WINDOW_RECOMMENDED,
};
use zkp::utils::{
    convertion::{address_to_fr, u256_to_fr},
    tree::{gadgets::leaf_hash::compute_leaf_hash, incremental_merkle_tree::IncrementalMerkleTree},
};

const TREE_HEIGHT: u32 = 64;

#[tokio::test(flavor = "multi_thread")]
async fn db_merkle_tree_tracks_history() -> Result<()> {
    let database = match TestDatabase::create("merkle_test").await {
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
    .context("failed to load migrations for merkle tree test")?;
    migrator
        .run(database.pool())
        .await
        .context("failed to run migrations for merkle tree test")?;

    let token_address = Address::from_slice(&[0x11; 20]);
    let verifier_address = Address::from_slice(&[0x22; 20]);
    let chain_id: i64 = 1337;

    let token_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO tokens (token_address, verifier_address, chain_id)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(token_address.as_slice())
    .bind(verifier_address.as_slice())
    .bind(chain_id)
    .fetch_one(database.pool())
    .await
    .context("failed to insert test token")?;

    let tree_config = DbMerkleTreeConfig::new(HISTORY_WINDOW_RECOMMENDED)?;
    let tree =
        DbIncrementalMerkleTree::new(database.pool().clone(), token_id, TREE_HEIGHT, tree_config)
            .await
            .context("failed to construct DbIncrementalMerkleTree")?;

    let mut reference = IncrementalMerkleTree::new(TREE_HEIGHT as usize);
    let mut leaves = Vec::new();
    let total_leaves = 120u64;

    for i in 0..total_leaves {
        let mut addr_bytes = [0u8; 20];
        addr_bytes[..8].copy_from_slice(&i.to_be_bytes());
        let address = Address::from_slice(&addr_bytes);
        let value = U256::from(i + 1);

        let append = tree
            .append_leaf(address, value)
            .await
            .with_context(|| format!("failed to append leaf {i}"))?;

        let ref_index = reference.insert(address, value);
        leaves.push((address, value));

        assert_eq!(append.index, i + 1, "index should increment sequentially");
        assert_eq!(append.leaf_index, i, "leaf index should match append order");
        assert_eq!(ref_index, i, "reference tree index mismatch");
        assert_eq!(
            append.root,
            reference.get_root(),
            "root mismatch vs reference"
        );
        assert_eq!(
            append.hash_chain, reference.hash_chain,
            "hash chain mismatch vs reference"
        );
    }

    let checkpoints = [1u64, 60, total_leaves];
    for &tree_index in checkpoints.iter() {
        let ref_tree = build_reference_tree(&leaves, tree_index as usize);
        let db_root = tree
            .root_at(tree_index)
            .await?
            .expect("expected stored root for index");
        assert_eq!(db_root, ref_tree.get_root(), "historical root mismatch");

        let db_hash_chain = tree
            .hash_chain_at(tree_index)
            .await?
            .expect("expected stored hash chain for index");
        assert_eq!(
            db_hash_chain, ref_tree.hash_chain,
            "historical hash chain mismatch"
        );
    }

    let mid_target = total_leaves - 20;
    let leaf_index = 10u64;
    let mid_proof = tree
        .prove(mid_target, leaf_index)
        .await
        .context("failed to fetch historical proof")?;
    let ref_mid = build_reference_tree(&leaves, mid_target as usize);
    let leaf = leaves[leaf_index as usize];
    let leaf_hash = compute_leaf_hash(address_to_fr(leaf.0), u256_to_fr(leaf.1));
    let computed_root = mid_proof.proof.get_root(leaf_hash, leaf_index);
    assert_eq!(
        computed_root,
        ref_mid.get_root(),
        "proof root should match reference for index {mid_target}"
    );
    assert_eq!(mid_proof.root, ref_mid.get_root(), "stored root mismatch");
    assert_eq!(
        mid_proof.hash_chain, ref_mid.hash_chain,
        "stored hash chain mismatch"
    );

    let batch_indices = [0u64, leaf_index, 25u64];
    let batch_proofs = tree
        .prove_many(mid_target, &batch_indices)
        .await
        .context("failed to fetch batch proofs")?;
    assert_eq!(
        batch_proofs.len(),
        batch_indices.len(),
        "batch proof count should match inputs"
    );
    for (proof, &idx) in batch_proofs.iter().zip(batch_indices.iter()) {
        assert_eq!(proof.target_index, mid_target, "batch proof index mismatch");
        assert_eq!(proof.leaf_index, idx, "batch proof leaf index mismatch");
        assert_eq!(
            proof.root,
            ref_mid.get_root(),
            "batch proof root mismatch for leaf {idx}"
        );
        assert_eq!(
            proof.hash_chain, ref_mid.hash_chain,
            "batch hash chain mismatch for leaf {idx}"
        );

        let batch_leaf = leaves[idx as usize];
        let batch_leaf_hash =
            compute_leaf_hash(address_to_fr(batch_leaf.0), u256_to_fr(batch_leaf.1));
        let batch_root = proof.proof.get_root(batch_leaf_hash, idx);
        assert_eq!(
            batch_root,
            ref_mid.get_root(),
            "batch computed root mismatch for leaf {idx}"
        );
    }

    let latest_proof = tree
        .prove(total_leaves, total_leaves - 1)
        .await
        .context("failed to fetch latest proof")?;
    let latest_leaf = leaves[(total_leaves - 1) as usize];
    let latest_leaf_hash =
        compute_leaf_hash(address_to_fr(latest_leaf.0), u256_to_fr(latest_leaf.1));
    let latest_ref = {
        let mut t = IncrementalMerkleTree::new(TREE_HEIGHT as usize);
        for (addr, value) in leaves.iter() {
            t.insert(*addr, *value);
        }
        t
    };
    assert_eq!(
        latest_proof
            .proof
            .get_root(latest_leaf_hash, total_leaves - 1),
        latest_ref.get_root(),
        "latest proof mismatch"
    );

    if total_leaves > HISTORY_WINDOW_RECOMMENDED {
        let boundary_index = total_leaves - HISTORY_WINDOW_RECOMMENDED;
        let boundary_proof = tree
            .prove(boundary_index, 0)
            .await
            .context("failed to fetch boundary proof")?;
        let boundary_ref = build_reference_tree(&leaves, boundary_index as usize);
        let boundary_leaf = leaves[0];
        let boundary_hash =
            compute_leaf_hash(address_to_fr(boundary_leaf.0), u256_to_fr(boundary_leaf.1));
        assert_eq!(
            boundary_proof.proof.get_root(boundary_hash, 0),
            boundary_ref.get_root(),
            "boundary proof mismatch"
        );

        let too_old = boundary_index.saturating_sub(1).max(1);
        let err = tree.prove(too_old, 0).await.unwrap_err();
        assert!(
            err.to_string().contains("retention window"),
            "expected retention window error, got {err:?}"
        );
    }

    database.cleanup().await?;
    Ok(())
}

fn build_reference_tree(leaves: &[(Address, U256)], upto: usize) -> IncrementalMerkleTree {
    let mut tree = IncrementalMerkleTree::new(TREE_HEIGHT as usize);
    for (i, (addr, value)) in leaves.iter().take(upto).enumerate() {
        let idx = tree.insert(*addr, *value);
        assert_eq!(
            idx, i as u64,
            "reference tree index mismatch while rebuilding snapshot"
        );
    }
    tree
}
