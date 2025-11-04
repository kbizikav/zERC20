use alloy::primitives::{Address, B256, U256};
use ark_bn254::Fr;
use ark_ff::{BigInteger as _, PrimeField as _};
use num_bigint::BigUint;
use std::convert::TryFrom;

pub fn fr_to_address(x: Fr) -> Address {
    // Ethereum addresses use the lower 20 bytes of the big-endian field element.
    let x_bytes = x.into_bigint().to_bytes_be()[12..32].to_vec();
    Address::from_slice(&x_bytes)
}

pub fn address_to_fr(x: Address) -> Fr {
    let x_bigint = BigUint::from_bytes_be(&x.0.0);
    x_bigint.into()
}

pub fn u256_to_fr(x: U256) -> Fr {
    let x_bigint = BigUint::from_bytes_be(&x.to_be_bytes_vec());
    x_bigint.into()
}

pub fn fr_to_u256(x: Fr) -> U256 {
    U256::from_be_slice(&x.into_bigint().to_bytes_be())
}

pub fn b256_to_fr(x: B256) -> Fr {
    let x_bigint = BigUint::from_bytes_be(&x.0);
    x_bigint.into()
}

pub fn fr_to_b256(x: Fr) -> B256 {
    let x_u256 = fr_to_u256(x);
    x_u256.into()
}

pub fn u256_to_usize(x: U256) -> Option<usize> {
    let x_bigint = BigUint::from_bytes_be(&x.to_be_bytes_vec());
    usize::try_from(&x_bigint).ok()
}
