use alloy::{
    primitives::{B256, keccak256},
    signers::{Signer as _, local::PrivateKeySigner},
};
use anyhow::{Context, Result};

pub const SEED_MESSAGE: &str = "zERC20 | Seed Derivation\n\nYou are signing to derive a private seed used ONLY to generate\none-time burn receiving addresses for zERC20.\n\nFacts:\n- Not a transaction; no gas or approvals.\n- Cannot move funds or grant permissions.\n- If this signature is exposed, privacy may be reduced\n  (burn addresses may become linkable). Funds remain safe.\n- Keep this signature private.\n\nDetails:\n- App: zERC20\n- Purpose: Seed for burn address derivation\n- Version: 1";

pub async fn compute_seed_from_signature(private_key: B256) -> Result<B256> {
    let signer = PrivateKeySigner::from_bytes(&private_key).context("invalid private key")?;
    let signature: [u8; 65] = signer
        .sign_message(SEED_MESSAGE.as_bytes())
        .await
        .context("failed to sign derivation message")?
        .into();
    Ok(keccak256(signature))
}
