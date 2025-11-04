use alloy::primitives::{Address, B256, keccak256};
use rand::{RngCore, rngs::OsRng};
use zkp::{
    circuits::burn_address::{find_pow_nonce, secret_from_nonce},
    utils::{
        convertion::{b256_to_fr, fr_to_b256},
        general_recipient::GeneralRecipient,
    },
};

const INVOICE_SECRET_DOMAIN: [u8; 4] = *b"isec";
const INVOICE_TWEAK_DOMAIN: [u8; 4] = *b"itwk";
const PAYMENT_ADVICE_SECRET_DOMAIN: [u8; 4] = *b"psec";
const PAYMENT_ADVICE_TWEAK_DOMAIN: [u8; 4] = *b"ptwk";
const SINGLE_FLAG_MASK: u8 = 0x80;

pub fn random_invoice_id(is_single: bool, chain_id: u64) -> B256 {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);

    if is_single {
        bytes[0] |= SINGLE_FLAG_MASK;
    } else {
        bytes[0] &= !SINGLE_FLAG_MASK;
    }

    // Embed the target chain ID in bytes [1..=8] (big-endian)
    let chain_bytes = chain_id.to_be_bytes();
    bytes[1..=8].copy_from_slice(&chain_bytes);

    B256::from_slice(&bytes)
}

pub fn is_single(invoice_id: B256) -> bool {
    (invoice_id.as_slice()[0] & SINGLE_FLAG_MASK) != 0
}

pub fn extract_chain_id(invoice_id: B256) -> u64 {
    let mut chain_bytes = [0u8; 8];
    chain_bytes.copy_from_slice(&invoice_id.as_slice()[1..=8]);
    u64::from_be_bytes(chain_bytes)
}

pub fn random_payment_advice_id() -> B256 {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    B256::from_slice(&bytes)
}

#[derive(Debug, Clone, Copy)]
pub struct SecretAndTweak {
    pub secret: B256,
    pub tweak: B256,
}

impl SecretAndTweak {
    // Derive secret and tweak for a single invoice
    pub fn single_invoice(
        invoice_id: B256,
        seed: B256,
        recipient_chain_id: u64,
        recipient_address: Address,
    ) -> Self {
        let base_secret = keccak256(
            [
                &INVOICE_SECRET_DOMAIN,
                seed.as_slice(),
                invoice_id.as_slice(),
            ]
            .concat(),
        );
        let tweak = keccak256(
            [
                &INVOICE_TWEAK_DOMAIN,
                seed.as_slice(),
                invoice_id.as_slice(),
            ]
            .concat(),
        );
        let secret =
            Self::pow_adjusted_secret(base_secret, tweak, recipient_chain_id, recipient_address);
        Self { secret, tweak }
    }

    // Derive secret and tweak for a batch invoice with sub IDs
    pub fn batch_invoice(
        invoice_id: B256,
        sub_id: u32,
        seed: B256,
        recipient_chain_id: u64,
        recipient_address: Address,
    ) -> Self {
        let sub_id_bytes = sub_id.to_be_bytes();
        let base_secret = keccak256(
            [
                &INVOICE_SECRET_DOMAIN,
                seed.as_slice(),
                invoice_id.as_slice(),
                sub_id_bytes.as_slice(),
            ]
            .concat(),
        );
        // Note: tweak does not depend on sub_id for batch invoices
        let tweak = keccak256(
            [
                &INVOICE_TWEAK_DOMAIN,
                seed.as_slice(),
                invoice_id.as_slice(),
            ]
            .concat(),
        );
        let secret =
            Self::pow_adjusted_secret(base_secret, tweak, recipient_chain_id, recipient_address);
        Self { secret, tweak }
    }

    // Derive secret and tweak for a payment advice
    pub fn payment_advice(
        payment_advice_id: B256,
        seed: B256,
        recipient_chain_id: u64,
        recipient_address: Address,
    ) -> Self {
        let base_secret = keccak256(
            [
                &PAYMENT_ADVICE_SECRET_DOMAIN,
                seed.as_slice(),
                payment_advice_id.as_slice(),
            ]
            .concat(),
        );
        let tweak = keccak256(
            [
                &PAYMENT_ADVICE_TWEAK_DOMAIN,
                seed.as_slice(),
                payment_advice_id.as_slice(),
            ]
            .concat(),
        );
        let secret =
            Self::pow_adjusted_secret(base_secret, tweak, recipient_chain_id, recipient_address);
        Self { secret, tweak }
    }

    fn pow_adjusted_secret(
        base_secret: B256,
        tweak: B256,
        recipient_chain_id: u64,
        recipient_address: Address,
    ) -> B256 {
        let gr = GeneralRecipient::new_evm(recipient_chain_id, recipient_address, tweak);
        let recipient_fr = gr.to_fr();
        let secret_seed_fr = b256_to_fr(base_secret);
        let nonce = find_pow_nonce(recipient_fr, secret_seed_fr);
        let secret_fr = secret_from_nonce(secret_seed_fr, nonce);
        fr_to_b256(secret_fr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_single_invoice_sets_flag() {
        for _ in 0..16 {
            let invoice_id = random_invoice_id(true, 1);
            assert!(is_single(invoice_id));
        }
    }

    #[test]
    fn random_batch_invoice_clears_flag() {
        for _ in 0..16 {
            let invoice_id = random_invoice_id(false, 1);
            assert!(!is_single(invoice_id));
        }
    }

    #[test]
    fn chain_id_round_trip() {
        let chain_id = 1_337_u64;
        let invoice_id = random_invoice_id(true, chain_id);
        assert_eq!(extract_chain_id(invoice_id), chain_id);
    }
}
