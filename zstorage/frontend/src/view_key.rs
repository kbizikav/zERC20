use crate::error::{Result as StealthResult, StealthError};
use candid::Principal;
use ic_cdk::management_canister::{VetKDCurve, VetKDKeyId};
use ic_vetkeys::MasterPublicKey;
use key_manager::context_for_address;

/// Derive the view public key bound to `address` using `icp_vetkeys` locally.
///
/// The derivation mirrors the on-chain `vetkd_public_key` lookup performed by the
/// key manager canister. It requires the key manager canister id, the vetKD key id
/// name (for example, `"key_1"` on mainnet or `"test_key_1"` in development),
/// and the 20-byte recipient address.
pub fn derive_view_public_key_offchain(
    key_manager_canister_id: Principal,
    address: [u8; 20],
    key_id_name: &str,
) -> StealthResult<Vec<u8>> {
    let key_id = VetKDKeyId {
        curve: VetKDCurve::Bls12_381_G2,
        name: key_id_name.to_string(),
    };

    let master_public_key = MasterPublicKey::for_mainnet_key(&key_id).ok_or_else(|| {
        StealthError::VetKdDerivation(format!(
            "unknown master public key for key id '{}'",
            key_id_name
        ))
    })?;

    let canister_public_key =
        master_public_key.derive_canister_key(key_manager_canister_id.as_slice());

    let context = context_for_address(&address);
    let derived_public_key = canister_public_key.derive_sub_key(&context);
    Ok(derived_public_key.serialize())
}
