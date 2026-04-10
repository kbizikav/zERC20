// SPDX-License-Identifier: BUSL-1.1

//! Exclusion tree for OFAC non-membership proofs.
//!
//! Converts a sorted list of sanctioned addresses into a Merkle tree of gaps,
//! enabling efficient non-membership proofs.
//!
//! # Exclusion Tree Structure
//!
//! Given k sanctioned addresses, we create k+1 gaps representing the "innocent"
//! address ranges. Each gap leaf is `poseidon2(start, end)`.
//!
//! Example with sanctions list [100, 200, 300]:
//! - Gap 0: (0, 100) - addresses 1-99 are innocent
//! - Gap 1: (100, 200) - addresses 101-199 are innocent
//! - Gap 2: (200, 300) - addresses 201-299 are innocent
//! - Gap 3: (300, MAX) - addresses 301+ are innocent
//!
//! To prove address X is NOT sanctioned, find the gap where `start < X < end`
//! and provide the Merkle proof for that gap leaf.

use crate::utils::poseidon::utils::poseidon2;
use crate::utils::tree::merkle_tree::{MerkleProof, MerkleTree};
use ark_bn254::Fr;
use ark_ff::PrimeField;
use num_bigint::BigUint;
use std::str::FromStr;

/// Maximum 160-bit address value (2^160 - 1)
fn max_address() -> Fr {
    // 2^160 - 1 = 1461501637330902918203684832716283019655932542975
    let max_160 = BigUint::from_str("1461501637330902918203684832716283019655932542975").unwrap();
    Fr::from(max_160)
}

/// A gap in the exclusion tree.
///
/// Represents a range (start, end) where any address X satisfying
/// `start < X < end` is proven to NOT be on the sanctions list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    /// Lower bound (exclusive for non-membership)
    pub start: Fr,
    /// Upper bound (exclusive for non-membership)
    pub end: Fr,
}

impl Gap {
    /// Create a new gap
    pub fn new(start: Fr, end: Fr) -> Self {
        Self { start, end }
    }

    /// Compute the leaf hash for this gap: poseidon2(start, end)
    pub fn leaf_hash(&self) -> Fr {
        poseidon2(self.start, self.end)
    }

    /// Returns true if `addr` falls strictly within this gap.
    ///
    /// An address is in the gap iff `start < addr < end`, meaning
    /// it is NOT a sanctioned address.
    pub fn contains(&self, addr: Fr) -> bool {
        // We need to compare field elements as integers
        // Since these are 160-bit addresses, they fit well within the field
        let start_bigint = self.start.into_bigint();
        let end_bigint = self.end.into_bigint();
        let addr_bigint = addr.into_bigint();

        addr_bigint > start_bigint && addr_bigint < end_bigint
    }
}

/// Exclusion tree for efficient non-membership proofs.
///
/// Wraps a Merkle tree where each leaf represents a gap between
/// sanctioned addresses.
#[derive(Clone, Debug)]
pub struct ExclusionTree {
    /// The gaps (sorted by start address)
    pub gaps: Vec<Gap>,
    /// Underlying Merkle tree
    tree: MerkleTree,
}

impl ExclusionTree {
    /// Build an exclusion tree from a sorted list of sanctioned addresses.
    ///
    /// # Arguments
    /// * `addresses` - Sorted list of sanctioned addresses as field elements
    /// * `height` - Tree height (must accommodate all gaps: 2^height >= addresses.len() + 1)
    ///
    /// # Panics
    /// Panics if addresses are not sorted or if height is insufficient.
    pub fn from_sorted_addresses(addresses: &[Fr], height: usize) -> Self {
        // Verify addresses are sorted
        for window in addresses.windows(2) {
            assert!(
                window[0].into_bigint() < window[1].into_bigint(),
                "Addresses must be strictly sorted"
            );
        }

        let gaps = Self::compute_gaps(addresses);
        assert!(
            gaps.len() <= (1 << height),
            "Tree height {} too small for {} gaps (need at least {})",
            height,
            gaps.len(),
            (gaps.len() as f64).log2().ceil() as usize
        );

        let mut tree = MerkleTree::new(height);
        for (i, gap) in gaps.iter().enumerate() {
            tree.update_leaf(i as u64, gap.leaf_hash());
        }

        Self { gaps, tree }
    }

    /// Parse Ethereum addresses from hex strings.
    ///
    /// Accepts addresses with or without "0x" prefix.
    /// Returns a sorted vector of field elements.
    ///
    /// # Example
    /// ```ignore
    /// let addrs = ExclusionTree::parse_addresses(&[
    ///     "0x1234567890abcdef1234567890abcdef12345678",
    ///     "0xabcdef1234567890abcdef1234567890abcdef12",
    /// ]);
    /// ```
    pub fn parse_addresses(hex_addresses: &[&str]) -> Vec<Fr> {
        let mut addresses: Vec<Fr> = hex_addresses
            .iter()
            .filter_map(|s| {
                let s = s.trim();
                if s.is_empty() {
                    return None;
                }
                let hex_str = s.strip_prefix("0x").unwrap_or(s);
                let big =
                    BigUint::parse_bytes(hex_str.as_bytes(), 16).expect("Invalid hex address");
                Some(Fr::from(big))
            })
            .collect();

        // Sort by integer value
        addresses.sort_by_key(|a| a.into_bigint());
        addresses
    }

    /// Load addresses from a file (one hex address per line).
    pub fn parse_addresses_from_file(path: &str) -> std::io::Result<Vec<Fr>> {
        let content = std::fs::read_to_string(path)?;
        let lines: Vec<&str> = content.lines().collect();
        Ok(Self::parse_addresses(&lines))
    }

    /// Compute gaps from a sorted list of sanctioned addresses.
    ///
    /// Given addresses [A, B, C], produces gaps:
    /// - (0, A): addresses before A
    /// - (A, B): addresses between A and B
    /// - (B, C): addresses between B and C
    /// - (C, MAX): addresses after C
    fn compute_gaps(sorted_addresses: &[Fr]) -> Vec<Gap> {
        let max_addr = max_address();

        if sorted_addresses.is_empty() {
            // Single gap covering entire address space
            return vec![Gap::new(Fr::from(0u64), max_addr)];
        }

        let mut gaps = Vec::with_capacity(sorted_addresses.len() + 1);

        // Gap before first sanctioned address: (0, first)
        gaps.push(Gap::new(Fr::from(0u64), sorted_addresses[0]));

        // Gaps between consecutive sanctioned addresses
        for window in sorted_addresses.windows(2) {
            gaps.push(Gap::new(window[0], window[1]));
        }

        // Gap after last sanctioned address: (last, MAX)
        gaps.push(Gap::new(*sorted_addresses.last().unwrap(), max_addr));

        gaps
    }

    /// Get the root hash of the exclusion tree.
    ///
    /// This root is used as a public input to the proof of innocence circuit.
    pub fn root(&self) -> Fr {
        self.tree.get_root()
    }

    /// Get the height of the tree.
    pub fn height(&self) -> usize {
        self.tree.height()
    }

    /// Get the number of gaps in the tree.
    pub fn num_gaps(&self) -> usize {
        self.gaps.len()
    }

    /// Find the gap containing an address and generate a non-membership proof.
    ///
    /// Returns `Some(ExclusionProof)` if the address is NOT sanctioned,
    /// or `None` if the address IS sanctioned (exists in the sanctions list).
    ///
    /// # Arguments
    /// * `addr` - The address to prove is not sanctioned
    pub fn prove_non_membership(&self, addr: Fr) -> Option<ExclusionProof> {
        for (index, gap) in self.gaps.iter().enumerate() {
            if gap.contains(addr) {
                let merkle_proof = self.tree.prove(index as u64);
                return Some(ExclusionProof {
                    start: gap.start,
                    end: gap.end,
                    gap_index: index as u64,
                    siblings: merkle_proof.siblings,
                });
            }
        }
        // Address is sanctioned (falls on a boundary between gaps)
        None
    }

    /// Check if an address is sanctioned (on the list).
    pub fn is_sanctioned(&self, addr: Fr) -> bool {
        self.prove_non_membership(addr).is_none()
    }

    /// Verify an exclusion proof against this tree's root.
    pub fn verify_proof(&self, addr: Fr, proof: &ExclusionProof) -> bool {
        // Check address is in the claimed gap
        let gap = Gap::new(proof.start, proof.end);
        if !gap.contains(addr) {
            return false;
        }

        // Verify Merkle proof
        let merkle_proof = MerkleProof {
            siblings: proof.siblings.clone(),
        };
        let computed_root = merkle_proof.get_root(gap.leaf_hash(), proof.gap_index);
        computed_root == self.root()
    }
}

/// Proof that an address is NOT in the sanctions list.
///
/// Contains the gap boundaries and Merkle proof needed to verify
/// non-membership in the proof of innocence circuit.
#[derive(Clone, Debug)]
pub struct ExclusionProof {
    /// Lower bound of the gap containing the address
    pub start: Fr,
    /// Upper bound of the gap containing the address
    pub end: Fr,
    /// Index of this gap in the exclusion tree
    pub gap_index: u64,
    /// Merkle proof siblings from leaf to root
    pub siblings: Vec<Fr>,
}

impl ExclusionProof {
    /// Convert siblings to fixed-size array for circuit input.
    ///
    /// # Panics
    /// Panics if siblings length doesn't match DEPTH.
    pub fn siblings_array<const DEPTH: usize>(&self) -> [Fr; DEPTH] {
        self.siblings
            .clone()
            .try_into()
            .expect("Siblings length must match DEPTH")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ff::AdditiveGroup;

    #[test]
    fn test_gap_contains() {
        let gap = Gap::new(Fr::from(100u64), Fr::from(200u64));

        // Inside gap
        assert!(gap.contains(Fr::from(150u64)));
        assert!(gap.contains(Fr::from(101u64)));
        assert!(gap.contains(Fr::from(199u64)));

        // On boundaries (sanctioned)
        assert!(!gap.contains(Fr::from(100u64)));
        assert!(!gap.contains(Fr::from(200u64)));

        // Outside gap
        assert!(!gap.contains(Fr::from(50u64)));
        assert!(!gap.contains(Fr::from(250u64)));
    }

    #[test]
    fn test_gap_leaf_hash() {
        let gap = Gap::new(Fr::from(100u64), Fr::from(200u64));
        let expected = poseidon2(Fr::from(100u64), Fr::from(200u64));
        assert_eq!(gap.leaf_hash(), expected);
    }

    #[test]
    fn test_compute_gaps_empty() {
        let gaps = ExclusionTree::compute_gaps(&[]);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].start, Fr::ZERO);
        assert_eq!(gaps[0].end, max_address());
    }

    #[test]
    fn test_compute_gaps_single() {
        let addresses = vec![Fr::from(1000u64)];
        let gaps = ExclusionTree::compute_gaps(&addresses);

        assert_eq!(gaps.len(), 2);
        assert_eq!(gaps[0].start, Fr::ZERO);
        assert_eq!(gaps[0].end, Fr::from(1000u64));
        assert_eq!(gaps[1].start, Fr::from(1000u64));
        assert_eq!(gaps[1].end, max_address());
    }

    #[test]
    fn test_compute_gaps_multiple() {
        let addresses = vec![Fr::from(100u64), Fr::from(200u64), Fr::from(300u64)];
        let gaps = ExclusionTree::compute_gaps(&addresses);

        assert_eq!(gaps.len(), 4);
        assert_eq!(gaps[0], Gap::new(Fr::ZERO, Fr::from(100u64)));
        assert_eq!(gaps[1], Gap::new(Fr::from(100u64), Fr::from(200u64)));
        assert_eq!(gaps[2], Gap::new(Fr::from(200u64), Fr::from(300u64)));
        assert_eq!(gaps[3], Gap::new(Fr::from(300u64), max_address()));
    }

    #[test]
    fn test_parse_addresses() {
        let hex = vec![
            "0x00000000000000000000000000000000000000ff",
            "0x0000000000000000000000000000000000000064", // 100
            "0x00000000000000000000000000000000000000c8", // 200
        ];
        let addresses = ExclusionTree::parse_addresses(&hex);

        // Should be sorted
        assert_eq!(addresses.len(), 3);
        assert_eq!(addresses[0], Fr::from(100u64));
        assert_eq!(addresses[1], Fr::from(200u64));
        assert_eq!(addresses[2], Fr::from(255u64));
    }

    #[test]
    fn test_parse_addresses_no_prefix() {
        let hex = vec!["0000000000000000000000000000000000000064"];
        let addresses = ExclusionTree::parse_addresses(&hex);
        assert_eq!(addresses[0], Fr::from(100u64));
    }

    #[test]
    fn test_parse_addresses_skips_empty() {
        let hex = vec!["0x64", "", "  ", "0xc8"];
        let addresses = ExclusionTree::parse_addresses(&hex);
        assert_eq!(addresses.len(), 2);
    }

    #[test]
    fn test_exclusion_tree_basic() {
        let addresses = vec![Fr::from(100u64), Fr::from(200u64), Fr::from(300u64)];
        let tree = ExclusionTree::from_sorted_addresses(&addresses, 4);

        assert_eq!(tree.num_gaps(), 4);
        assert_eq!(tree.height(), 4);

        // Test non-sanctioned addresses
        assert!(tree.prove_non_membership(Fr::from(50u64)).is_some());
        assert!(tree.prove_non_membership(Fr::from(150u64)).is_some());
        assert!(tree.prove_non_membership(Fr::from(250u64)).is_some());
        assert!(tree.prove_non_membership(Fr::from(400u64)).is_some());

        // Test sanctioned addresses (on boundaries)
        assert!(tree.prove_non_membership(Fr::from(100u64)).is_none());
        assert!(tree.prove_non_membership(Fr::from(200u64)).is_none());
        assert!(tree.prove_non_membership(Fr::from(300u64)).is_none());
    }

    #[test]
    fn test_exclusion_tree_is_sanctioned() {
        let addresses = vec![Fr::from(100u64), Fr::from(200u64)];
        let tree = ExclusionTree::from_sorted_addresses(&addresses, 4);

        assert!(!tree.is_sanctioned(Fr::from(50u64)));
        assert!(!tree.is_sanctioned(Fr::from(150u64)));
        assert!(tree.is_sanctioned(Fr::from(100u64)));
        assert!(tree.is_sanctioned(Fr::from(200u64)));
    }

    #[test]
    fn test_exclusion_proof_verification() {
        let addresses = vec![Fr::from(100u64), Fr::from(200u64), Fr::from(300u64)];
        let tree = ExclusionTree::from_sorted_addresses(&addresses, 4);

        let addr = Fr::from(150u64);
        let proof = tree.prove_non_membership(addr).unwrap();

        // Verify proof
        assert!(tree.verify_proof(addr, &proof));

        // Wrong address should fail
        assert!(!tree.verify_proof(Fr::from(250u64), &proof));
    }

    #[test]
    fn test_exclusion_proof_siblings_array() {
        let addresses = vec![Fr::from(100u64)];
        let tree = ExclusionTree::from_sorted_addresses(&addresses, 4);

        let proof = tree.prove_non_membership(Fr::from(50u64)).unwrap();
        let siblings: [Fr; 4] = proof.siblings_array();
        assert_eq!(siblings.len(), 4);
    }

    #[test]
    fn test_real_addresses() {
        // Test with realistic Ethereum addresses
        let hex = vec![
            "0x04dba1194ee10112fe6c3207c0687def0e78bacf",
            "0x08723392ed15743cc38513c4925f5e6be5c17243",
            "0xfec8a60023265364d066a1212fde3930f6ae8da7",
        ];
        let addresses = ExclusionTree::parse_addresses(&hex);
        let tree = ExclusionTree::from_sorted_addresses(&addresses, 4);

        assert_eq!(tree.num_gaps(), 4);

        // Random address should be provable (unless it happens to match a sanctioned one)
        let random_addr =
            ExclusionTree::parse_addresses(&["0x1234567890abcdef1234567890abcdef12345678"])[0];
        let proof = tree.prove_non_membership(random_addr);
        assert!(proof.is_some());

        // Sanctioned address should not be provable
        let sanctioned = addresses[0];
        assert!(tree.prove_non_membership(sanctioned).is_none());
    }

    #[test]
    #[should_panic(expected = "strictly sorted")]
    fn test_unsorted_addresses_panic() {
        let addresses = vec![Fr::from(200u64), Fr::from(100u64)]; // Not sorted
        ExclusionTree::from_sorted_addresses(&addresses, 4);
    }

    #[test]
    #[should_panic(expected = "too small")]
    fn test_insufficient_height_panic() {
        let addresses: Vec<Fr> = (0..20).map(|i| Fr::from(i * 100u64)).collect();
        // 21 gaps need at least height 5 (2^5 = 32), but we provide 3 (2^3 = 8)
        ExclusionTree::from_sorted_addresses(&addresses, 3);
    }
}
