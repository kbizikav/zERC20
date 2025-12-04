use std::collections::HashMap;

use crate::utils::convertion::{address_to_fr, u256_to_fr};
use crate::utils::tree::gadgets::hash_chain::hash_chain;
use crate::utils::tree::gadgets::leaf_hash::compute_leaf_hash;
use crate::utils::tree::merkle_tree::{MerkleProof, MerkleTree};
use alloy::primitives::{Address, U256};
use ark_bn254::Fr;
use thiserror::Error;

const MAX_TREE_HEIGHT: u32 = 127;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IncrementalMerkleTreeError {
    #[error("tree height {height} exceeds supported limit of {max_height}")]
    HeightTooLarge { height: usize, max_height: usize },
    #[error(
        "incremental merkle tree full: height {height} supports {capacity} leaves, attempted index {index}"
    )]
    TreeFull {
        height: usize,
        capacity: u128,
        index: u64,
    },
}

pub type Result<T> = std::result::Result<T, IncrementalMerkleTreeError>;

pub struct Leaf {
    pub from: Address,
    pub to: Address,
    pub value: U256,
}

impl Leaf {
    pub fn hash(&self) -> Fr {
        let from_fr = address_to_fr(self.from);
        let address_fr = address_to_fr(self.to);
        let value_fr = u256_to_fr(self.value);
        compute_leaf_hash(from_fr, address_fr, value_fr)
    }
}

pub struct UpdateProof {
    pub index: u64,
    pub old_leaf: Leaf,
    pub new_leaf: Leaf,
    pub merkle_proof: MerkleProof,
}

pub struct IncrementalMerkleTree {
    pub tree: MerkleTree,
    pub index: u64,
    pub hash_chain: U256,
    pub leaves: HashMap<u64, Leaf>,
    pub address_to_indices: HashMap<Address, Vec<u64>>,
}

impl IncrementalMerkleTree {
    pub fn new(height: usize) -> Self {
        Self {
            tree: MerkleTree::new(height),
            index: 0,
            hash_chain: U256::ZERO,
            leaves: HashMap::new(),
            address_to_indices: HashMap::new(),
        }
    }

    pub fn insert(&mut self, from: Address, to: Address, value: U256) -> Result<u64> {
        let leaf = Leaf { from, to, value };
        let leaf_hash = leaf.hash();
        let index = self.index;
        let height = self.tree.height();
        let max_leaves = max_leaf_capacity(height)?;
        if u128::from(index) >= max_leaves {
            return Err(IncrementalMerkleTreeError::TreeFull {
                height,
                capacity: max_leaves,
                index,
            });
        }
        self.tree.update_leaf(index, leaf_hash);
        self.hash_chain = hash_chain(self.hash_chain, leaf.from, leaf.to, leaf.value);
        self.leaves.insert(index, leaf);
        self.address_to_indices.entry(to).or_default().push(index);
        self.index = self
            .index
            .checked_add(1)
            .expect("leaf capacity check prevents index overflow");
        Ok(index)
    }

    pub fn get_root(&self) -> Fr {
        self.tree.get_root()
    }

    pub fn prove(&self, index: u64) -> MerkleProof {
        self.tree.prove(index)
    }
}

fn max_leaf_capacity(height: usize) -> Result<u128> {
    let height_u32 =
        u32::try_from(height).map_err(|_| IncrementalMerkleTreeError::HeightTooLarge {
            height,
            max_height: MAX_TREE_HEIGHT as usize,
        })?;
    if height_u32 > MAX_TREE_HEIGHT {
        return Err(IncrementalMerkleTreeError::HeightTooLarge {
            height,
            max_height: MAX_TREE_HEIGHT as usize,
        });
    }
    let theoretical = 1u128 << height_u32;
    let practical_limit = theoretical.min(u128::from(u64::MAX));
    Ok(practical_limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{nova::constants::TRANSFER_TREE_HEIGHT, utils::convertion::fr_to_u256};
    use core::str::FromStr;

    #[test]
    fn insert_rejects_capacity_overflow() {
        let mut tree = IncrementalMerkleTree::new(2);
        for _ in 0..4 {
            tree.insert(Address::ZERO, Address::ZERO, U256::from(1u64))
                .expect("within capacity");
        }
        let err = tree
            .insert(Address::ZERO, Address::ZERO, U256::from(1u64))
            .unwrap_err();
        assert!(matches!(
            err,
            IncrementalMerkleTreeError::TreeFull { capacity, .. } if capacity == 4
        ));
        assert_eq!(tree.index, 4);
        assert_eq!(tree.address_to_indices[&Address::ZERO].len(), 4);
    }

    #[test]
    fn initial_transfer_root_matches_expected() {
        let tree = IncrementalMerkleTree::new(TRANSFER_TREE_HEIGHT);
        let root = tree.get_root();
        let root_u256 = fr_to_u256(root);
        let expected = U256::from_str(
            "8687547638004116013653730449839507042090717944911454416140763808366589487233",
        )
        .expect("decimal fits in U256");
        assert_eq!(
            root_u256, expected,
            "computed initial transfer root {} does not match expected",
            root_u256
        );
    }
}
