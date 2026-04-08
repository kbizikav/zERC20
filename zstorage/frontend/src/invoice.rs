// SPDX-License-Identifier: BUSL-1.1

use alloy_primitives::utils::eip191_message;

/// Build the human-readable message string (without EIP-191 prefix) for invoice signing.
pub fn invoice_message_text(invoice_id: &[u8; 32], tag: &str) -> String {
    format!(
        "ICP Stealth Invoice Submission:\ninvoice_id: 0x{}\ntag: {}",
        hex::encode(invoice_id),
        tag
    )
}

/// Build the EIP-191-prefixed message bytes that should be signed for invoice submission.
pub fn invoice_signature_message(invoice_id: &[u8; 32], tag: &str) -> Vec<u8> {
    let text = invoice_message_text(invoice_id, tag);
    eip191_message(text.as_bytes())
}
