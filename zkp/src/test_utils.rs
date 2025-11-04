use crate::utils::poseidon::utils::circom_poseidon_hash;
use ark_crypto_primitives::sponge::{Absorb, poseidon::PoseidonConfig};
use ark_ff::{BigInteger, PrimeField};

/// Truncate a field element to 160 bits by keeping its low-order 20 bytes.
pub fn truncate_to_160_bits<F: PrimeField>(value: F) -> F {
    let mut bytes = value.into_bigint().to_bytes_le();
    bytes.truncate(20);
    F::from_le_bytes_mod_order(&bytes)
}

/// Recompute a Poseidon Merkle root given a leaf, its index, and authentication path.
pub fn merkle_root_from_path<F: PrimeField + Absorb>(
    config: &PoseidonConfig<F>,
    index: u64,
    leaf: F,
    siblings: &[F],
) -> F {
    let mut current = leaf;
    for (depth, sibling) in siblings.iter().enumerate() {
        let bit = (index >> depth) & 1;
        let (left, right) = if bit == 0 {
            (current, *sibling)
        } else {
            (*sibling, current)
        };
        current = circom_poseidon_hash(config, &[left, right]);
    }
    current
}
