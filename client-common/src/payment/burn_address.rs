use alloy::primitives::{Address, B256};
use anyhow::{Context, Result, ensure};
use zkp::{
    circuits::burn_address::compute_burn_address_from_secret,
    utils::{
        convertion::{b256_to_fr, fr_to_address},
        general_recipient::GeneralRecipient,
    },
};

use crate::payment::invoice::SecretAndTweak;

#[derive(Debug, Clone)]
pub struct FullBurnAddress {
    pub gr: GeneralRecipient,
    pub secret: B256,
}

const FULL_BURN_ADDRESS_VERSION: u8 = 1;
const FULL_BURN_ADDRESS_SERIALIZED_LEN: usize = 1 + 8 + 32 + 32 + 32;

impl FullBurnAddress {
    pub fn new(
        recipient_chain_id: u64,
        recipient_address: Address,
        secret_and_tweak: &SecretAndTweak,
    ) -> Result<Self> {
        let gr = GeneralRecipient::new_evm(
            recipient_chain_id,
            recipient_address,
            secret_and_tweak.tweak,
        );
        let recipient_fr = gr.to_fr();
        let secret_fr = b256_to_fr(secret_and_tweak.secret);
        compute_burn_address_from_secret(recipient_fr, secret_fr)
            .context("burn secret must satisfy PoW")?;
        Ok(Self {
            gr,
            secret: secret_and_tweak.secret,
        })
    }

    pub fn burn_address(&self) -> Result<Address> {
        let recipient_fr = self.gr.to_fr();
        let burn_address_fr =
            compute_burn_address_from_secret(recipient_fr, b256_to_fr(self.secret))
                .context("burn secret must satisfy PoW")?;
        Ok(fr_to_address(burn_address_fr))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(FULL_BURN_ADDRESS_SERIALIZED_LEN);
        bytes.push(FULL_BURN_ADDRESS_VERSION);
        bytes.extend_from_slice(&self.gr.chain_id.to_be_bytes());
        bytes.extend_from_slice(self.gr.address.as_slice());
        bytes.extend_from_slice(self.gr.tweak.as_slice());
        bytes.extend_from_slice(self.secret.as_slice());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() == FULL_BURN_ADDRESS_SERIALIZED_LEN,
            "FullBurnAddress expects {} bytes, got {}",
            FULL_BURN_ADDRESS_SERIALIZED_LEN,
            bytes.len()
        );

        ensure!(
            bytes[0] == FULL_BURN_ADDRESS_VERSION,
            "unsupported FullBurnAddress version {}",
            bytes[0]
        );

        let mut chain_id_bytes = [0u8; 8];
        chain_id_bytes.copy_from_slice(&bytes[1..9]);
        let chain_id = u64::from_be_bytes(chain_id_bytes);

        let address = B256::from_slice(&bytes[9..41]);
        let tweak = B256::from_slice(&bytes[41..73]);
        let secret = B256::from_slice(&bytes[73..105]);

        let gr = GeneralRecipient {
            chain_id,
            address,
            tweak,
        };

        Ok(Self { gr, secret })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payment::invoice::SecretAndTweak;
    use anyhow::Result;

    #[test]
    fn round_trip_serialize() -> Result<()> {
        let recipient_address = Address::from_slice(&[0x11; 20]);
        let chain_id = 10u64;
        let payment_advice_id = B256::from_slice(&[0xAA; 32]);
        let seed = B256::from_slice(&[0xBB; 32]);
        let secret_and_tweak =
            SecretAndTweak::payment_advice(payment_advice_id, seed, chain_id, recipient_address);

        let burn = FullBurnAddress::new(chain_id, recipient_address, &secret_and_tweak)?;
        let encoded = burn.to_bytes();
        let decoded = FullBurnAddress::from_bytes(&encoded)?;

        assert_eq!(decoded.gr.chain_id, burn.gr.chain_id);
        assert_eq!(decoded.gr.address, burn.gr.address);
        assert_eq!(decoded.gr.tweak, burn.gr.tweak);
        assert_eq!(decoded.secret, burn.secret);
        Ok(())
    }

    #[test]
    fn invalid_version_fails() -> Result<()> {
        let recipient_address = Address::from_slice(&[0x22; 20]);
        let chain_id = 77u64;
        let payment_advice_id = B256::from_slice(&[0xCC; 32]);
        let seed = B256::from_slice(&[0xDD; 32]);
        let secret_and_tweak =
            SecretAndTweak::payment_advice(payment_advice_id, seed, chain_id, recipient_address);

        let burn = FullBurnAddress::new(chain_id, recipient_address, &secret_and_tweak)?;
        let mut encoded = burn.to_bytes();
        encoded[0] = 0;

        assert!(FullBurnAddress::from_bytes(&encoded).is_err());
        Ok(())
    }

    #[test]
    fn invalid_length_fails() {
        let too_short = vec![FULL_BURN_ADDRESS_VERSION; FULL_BURN_ADDRESS_SERIALIZED_LEN - 1];
        assert!(FullBurnAddress::from_bytes(&too_short).is_err());
    }
}
