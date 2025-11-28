use std::collections::HashMap;

use crate::utils::convertion::{address_to_fr, u256_to_fr};
use crate::utils::tree::gadgets::hash_chain::hash_chain;
use crate::utils::tree::gadgets::leaf_hash::compute_leaf_hash;
use crate::utils::tree::merkle_tree::{MerkleProof, MerkleTree};
use alloy::primitives::{Address, U256};
use ark_bn254::Fr;

pub struct Leaf {
    pub address: Address,
    pub value: U256,
}

impl Leaf {
    pub fn hash(&self) -> Fr {
        let address_fr = address_to_fr(self.address);
        let value_fr = u256_to_fr(self.value);
        compute_leaf_hash(address_fr, value_fr)
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

    pub fn insert(&mut self, address: Address, value: U256) -> u64 {
        let leaf = Leaf { address, value };
        let leaf_hash = leaf.hash();
        let index = self.index;
        self.tree.update_leaf(index, leaf_hash);
        self.hash_chain = hash_chain(self.hash_chain, leaf.address, leaf.value);
        self.leaves.insert(index, leaf);
        self.address_to_indices
            .entry(address)
            .or_default()
            .push(index);
        self.index += 1;
        index
    }

    pub fn get_root(&self) -> Fr {
        self.tree.get_root()
    }

    pub fn prove(&self, index: u64) -> MerkleProof {
        self.tree.prove(index)
    }
}
