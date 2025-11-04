use alloy_primitives::utils::eip191_message;

/// Build the human-readable message string (without EIP-191 prefix) for invoice signing.
pub fn invoice_message_text(invoice_id: &[u8; 32]) -> String {
    format!(
        "ICP Stealth Invoice Submission:\ninvoice_id: 0x{}",
        hex::encode(invoice_id)
    )
}

/// Build the EIP-191-prefixed message bytes that should be signed for invoice submission.
pub fn invoice_signature_message(invoice_id: &[u8; 32]) -> Vec<u8> {
    let text = invoice_message_text(invoice_id);
    eip191_message(text.as_bytes())
}
