use alloy::primitives::{Address, B256, U256};
use ark_bn254::Fr;
use sha2::{Digest as _, Sha256};

use crate::utils::convertion::u256_to_fr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeneralRecipient {
    // The chain ID of the recipient
    pub chain_id: u64,

    // The address of the recipient.
    // We use B256 instead of Address to adapt to non-EVM chains.
    pub address: B256,

    // A tweak to change the recipient and prevent generating long withdraw proofs.
    pub tweak: B256,
}

impl GeneralRecipient {
    pub fn new_evm(chain_id: u64, address: Address, tweak: B256) -> Self {
        Self {
            chain_id,
            address: address.into_word(),
            tweak,
        }
    }

    // The version byte to use in the U256 representation.
    // This can be used to distinguish different formats in the future.
    pub fn version() -> u8 {
        1
    }

    pub fn to_u256(&self) -> U256 {
        // Compute SHA256(chain_id || address || tweak)
        let mut digest: [u8; 32] = Sha256::digest(
            [
                self.chain_id.to_be_bytes().as_ref(),
                self.address.as_ref(),
                self.tweak.as_ref(),
            ]
            .concat(),
        )
        .into();
        // replace the most significant byte with version
        digest[0] = Self::version();
        U256::from_be_slice(&digest)
    }

    pub fn to_fr(&self) -> Fr {
        u256_to_fr(self.to_u256())
    }
}

#[cfg(test)]
mod tests {
    use super::GeneralRecipient;
    use alloy::primitives::{B256, U256};
    use sha2::{Digest, Sha256};

    fn sample_recipient() -> GeneralRecipient {
        let chain_id = 42u64;
        let address = B256::from([0x11u8; 32]);
        let tweak = B256::from([0x22u8; 32]);
        GeneralRecipient {
            chain_id,
            address,
            tweak,
        }
    }

    #[test]
    fn to_u256_sets_version_byte_and_matches_sha256() {
        let recipient = sample_recipient();
        let mut digest: [u8; 32] = Sha256::digest(
            [
                recipient.chain_id.to_be_bytes().as_ref(),
                recipient.address.as_ref(),
                recipient.tweak.as_ref(),
            ]
            .concat(),
        )
        .into();
        digest[0] = GeneralRecipient::version();
        let expected = U256::from_be_slice(&digest);

        assert_eq!(recipient.to_u256(), expected);
    }
}
