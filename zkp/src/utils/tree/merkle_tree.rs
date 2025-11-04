use crate::utils::poseidon::utils::poseidon2;
use ark_bn254::Fr;
use ark_ff::AdditiveGroup;
use std::collections::HashMap;

use super::bit_path::BitPath;

/// A Merkle tree that only keeps non-zero nodes. It has zero_hashes that hold the hash of each
/// level of empty leaves.
#[derive(Clone, Debug)]
pub struct MerkleTree {
    height: usize,
    node_hashes: HashMap<BitPath, Fr>,
    zero_hashes: Vec<Fr>,
}

impl MerkleTree {
    pub fn new(height: usize) -> Self {
        // zero_hashes = [H(zero_leaf), H(H(zero_leaf), H(zero_leaf)), ...]
        let mut zero_hashes = vec![];
        let mut h = Fr::ZERO;
        zero_hashes.push(h);
        for _ in 0..height {
            h = poseidon2(h, h);
            zero_hashes.push(h);
        }
        Self {
            height,
            node_hashes: HashMap::new(),
            zero_hashes,
        }
    }

    pub fn height(&self) -> usize {
        self.height
    }

    fn get_node_hash(&self, path: BitPath) -> Fr {
        match self.node_hashes.get(&path) {
            Some(h) => *h,
            None => self.zero_hashes[self.height - path.len() as usize],
        }
    }

    pub fn get_root(&self) -> Fr {
        self.get_node_hash(BitPath::default())
    }

    pub fn update_leaf(&mut self, index: u64, leaf_hash: Fr) {
        let mut path = BitPath::new(self.height as u32, index);
        let mut h = leaf_hash;
        self.node_hashes.insert(path, h);
        for _ in 0..self.height {
            let sibling = self.get_node_hash(path.sibling());
            h = if path.pop().unwrap() {
                poseidon2(sibling, h)
            } else {
                poseidon2(h, sibling)
            };
            self.node_hashes.insert(path, h);
        }
    }

    pub fn prove(&self, index: u64) -> MerkleProof {
        let mut path = BitPath::new(self.height as u32, index);
        let mut siblings = vec![];
        for _ in 0..self.height {
            siblings.push(self.get_node_hash(path.sibling()));
            path.pop();
        }
        MerkleProof { siblings }
    }
}

#[derive(Clone, Debug)]
pub struct MerkleProof {
    pub siblings: Vec<Fr>,
}

impl MerkleProof {
    pub fn dummy(height: usize) -> Self {
        Self {
            siblings: vec![Fr::default(); height],
        }
    }

    pub fn height(&self) -> usize {
        self.siblings.len()
    }

    pub fn get_root(&self, leaf_hash: Fr, index: u64) -> Fr {
        let mut path = BitPath::new(self.height() as u32, index);
        let mut state = leaf_hash;
        for sibling in self.siblings.iter() {
            let bit = path.pop().unwrap();
            state = if bit {
                poseidon2(*sibling, state)
            } else {
                poseidon2(state, *sibling)
            }
        }
        state
    }

    pub fn extend(&self, other: &Self) -> Self {
        let mut siblings = self.siblings.clone();
        siblings.extend_from_slice(&other.siblings);
        Self { siblings }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ff::UniformRand as _;
    use rand::Rng;

    #[test]
    fn test_merkle_tree_new() {
        // Test with different heights
        let heights = [1, 5, 10, 20];
        for height in heights {
            let tree = MerkleTree::new(height);
            assert_eq!(tree.height(), height);
            assert_eq!(tree.zero_hashes.len(), height + 1);
            assert!(tree.node_hashes.is_empty());
        }
    }

    #[test]
    fn test_merkle_tree_prove_simple() {
        let mut tree = MerkleTree::new(20);

        let leaf_hash = Fr::from(42u64);
        let index = 1u64;
        tree.update_leaf(index, leaf_hash);

        let root = tree.get_root();
        let proof = tree.prove(index);
        let calculated_root = proof.get_root(leaf_hash, index);
        assert_eq!(calculated_root, root);
    }

    #[test]
    fn test_merkle_tree_get_root() {
        let height = 10;
        let tree = MerkleTree::new(height);

        // Root of empty tree should match the top zero hash
        assert_eq!(tree.get_root(), tree.zero_hashes[height]);

        // After updates, root should change
        let mut tree = MerkleTree::new(height);
        let mut rng = rand::thread_rng();
        let index: u64 = rng.gen_range(0..1 << height);
        let leaf_hash = Fr::rand(&mut rng);

        let empty_root = tree.get_root();
        tree.update_leaf(index, leaf_hash);
        let updated_root = tree.get_root();

        assert_ne!(empty_root, updated_root);
    }

    #[test]
    fn test_merkle_tree_update_prove_verify() {
        let mut rng = rand::thread_rng();
        let height = 10;
        let mut tree = MerkleTree::new(height);
        let mut leaf_hashes = HashMap::new();

        for _ in 0..10 {
            let index: u64 = rng.gen_range(0..1 << height);
            let leaf_hash = Fr::rand(&mut rng);
            tree.update_leaf(index, leaf_hash);
            leaf_hashes.insert(index, leaf_hash);
            let proof = tree.prove(index);
            let calc_root = proof.get_root(leaf_hash, index);
            assert_eq!(calc_root, tree.get_root());
        }

        for (index, leaf_hash) in leaf_hashes.iter() {
            let proof = tree.prove(*index);
            let calc_root = proof.get_root(*leaf_hash, *index);
            assert_eq!(calc_root, tree.get_root());
        }
    }
}
