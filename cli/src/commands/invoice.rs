use std::collections::HashMap;

use alloy::primitives::{Address, B256, keccak256};
use anyhow::{Context, Result, ensure};
use client_common::{
    contracts::{utils::get_address_from_private_key, verifier::VerifierContract},
    indexer::HttpIndexerClient,
    payment::{
        burn_address::FullBurnAddress,
        invoice::{SecretAndTweak, extract_chain_id, is_single, random_invoice_id},
        seed::compute_seed_from_signature,
    },
    teleport::{
        aggregation_tree::{AggregationTreeState, fetch_aggregation_tree_state},
        events::{EventsWithEligibility, fetch_transfer_events, separate_events_by_eligibility},
    },
    tokens::{HubEntry, TokenEntry},
};
use zkp::utils::general_recipient::GeneralRecipient;

use crate::{
    CommonArgs, InvoiceIssueArgs, InvoiceListArgs, InvoiceReceiveArgs, build_indexer_client,
    commands::{
        shared::{
            build_erc20, build_hub, build_stealth_client, build_verifier, find_token_by_chain,
        },
        teleport::{RedeemResult, print_events, redeem_transfers},
    },
};
use hex;
use k256::{FieldBytes, ecdsa::SigningKey};
use stealth_client::types::InvoiceSubmission;
use storage::invoice_signature_message;

pub const NUM_BATCH_INVOICES: usize = 10;

struct InvoiceReceiveContext {
    verifier: VerifierContract,
    indexer: HttpIndexerClient,
    gr: GeneralRecipient,
    aggregation_tree_state: AggregationTreeState,
    separated_events: HashMap<u64, EventsWithEligibility>,
    burn_address_to_secret_and_tweak: HashMap<Address, SecretAndTweak>,
}

async fn build_invoice_receive_context(
    common_args: &CommonArgs,
    args: &InvoiceReceiveArgs,
    token_entries: &[TokenEntry],
    hub: Option<&HubEntry>,
    private_key: B256,
    context_label: &str,
) -> Result<Option<InvoiceReceiveContext>> {
    let hub_entry =
        hub.ok_or_else(|| anyhow::anyhow!("hub entry is required to use {}", context_label))?;
    let hub = build_hub(hub_entry)?;
    let token_entry = find_token_by_chain(token_entries, args.chain_id)?.clone();
    let verifier = build_verifier(&token_entry)?;

    let seed = compute_seed_from_signature(private_key)
        .await
        .context("failed to derive seed from signature")?;
    let recipient_address = get_address_from_private_key(private_key);
    let recipient_chain_id = args.chain_id;

    let is_single = is_single(args.invoice_id);
    let mut burn_address_to_secret_and_tweak: HashMap<Address, SecretAndTweak> = HashMap::new();

    if is_single {
        let secret_and_tweak = SecretAndTweak::single_invoice(
            args.invoice_id,
            seed,
            recipient_chain_id,
            recipient_address,
        );
        let burn_payload =
            FullBurnAddress::new(recipient_chain_id, recipient_address, &secret_and_tweak)?;
        let burn_address = burn_payload
            .burn_address()
            .context("failed to derive burn address for single invoice")?;
        burn_address_to_secret_and_tweak.insert(burn_address, secret_and_tweak.clone());
    } else {
        for sub_id in 0..NUM_BATCH_INVOICES {
            let secret_and_tweak = SecretAndTweak::batch_invoice(
                args.invoice_id,
                sub_id as u32,
                seed,
                recipient_chain_id,
                recipient_address,
            );
            let burn_payload =
                FullBurnAddress::new(recipient_chain_id, recipient_address, &secret_and_tweak)?;
            let burn_address = burn_payload.burn_address().with_context(|| {
                format!(
                    "failed to derive burn address for batch invoice sub {}",
                    sub_id
                )
            })?;
            burn_address_to_secret_and_tweak.insert(burn_address, secret_and_tweak.clone());
        }
    }

    if burn_address_to_secret_and_tweak.is_empty() {
        return Ok(None);
    }

    let tweak = burn_address_to_secret_and_tweak
        .values()
        .next()
        .expect("at least one burn address")
        .tweak;
    let gr = GeneralRecipient::new_evm(recipient_chain_id, recipient_address, tweak);
    let burn_addresses: Vec<_> = burn_address_to_secret_and_tweak.keys().cloned().collect();
    let token_clients: Vec<_> = token_entries
        .iter()
        .map(build_erc20)
        .collect::<Result<Vec<_>>>()?;
    let indexer = build_indexer_client(common_args, context_label)?;

    let events_map = fetch_transfer_events(
        &indexer,
        Some(common_args.indexer_fetch_limit),
        token_entries,
        &token_clients,
        &burn_addresses,
    )
    .await
    .context("failed to fetch transfer events for invoice redemption")?;

    let aggregation_tree_state =
        fetch_aggregation_tree_state(common_args.event_block_span, &verifier, &hub)
            .await
            .context("failed to fetch aggregation tree state")?;

    let separated_events = separate_events_by_eligibility(&aggregation_tree_state, &events_map)?;

    Ok(Some(InvoiceReceiveContext {
        verifier,
        indexer,
        gr,
        aggregation_tree_state,
        separated_events,
        burn_address_to_secret_and_tweak,
    }))
}

pub async fn list(
    common_args: &CommonArgs,
    args: &InvoiceListArgs,
    tokens: &[TokenEntry],
    private_key: B256,
) -> Result<()> {
    let entry = find_token_by_chain(tokens, args.chain_id)?;
    let client = build_stealth_client(common_args)
        .await
        .context("failed to construct stealth canister client")?;
    let owner = args
        .owner
        .unwrap_or_else(|| get_address_from_private_key(private_key));
    let raw_invoice_ids = client
        .list_invoices(address_to_array(owner))
        .await
        .with_context(|| format!("failed to load invoice ids for {}", owner))?;
    let invoice_ids = decode_invoice_ids(raw_invoice_ids)?;
    let invoice_ids: Vec<_> = invoice_ids
        .into_iter()
        .filter(|invoice_id| extract_chain_id(*invoice_id) == entry.chain_id)
        .collect();

    if invoice_ids.is_empty() {
        println!("No invoices found for {}", owner);
    } else {
        println!("Invoices for {} (chain {}):", owner, entry.chain_id);
        for (idx, invoice_id) in invoice_ids.iter().enumerate() {
            println!(
                "{:<3}0x{}{}",
                idx,
                hex::encode(invoice_id.as_slice()),
                if is_single(*invoice_id) {
                    " (single)"
                } else {
                    " (batch)"
                }
            );
        }
    }

    Ok(())
}

pub async fn issue(
    common_args: &CommonArgs,
    args: &InvoiceIssueArgs,
    tokens: &[TokenEntry],
    private_key: B256,
) -> Result<()> {
    let entry = find_token_by_chain(tokens, args.chain_id)?;
    let client = build_stealth_client(common_args)
        .await
        .context("failed to construct stealth canister client")?;
    let seed = compute_seed_from_signature(private_key)
        .await
        .context("failed to derive seed from signature")?;
    let recipient = args
        .recipient
        .unwrap_or_else(|| get_address_from_private_key(private_key));
    let recipient_chain_id = entry.chain_id;

    let recipient_bytes = address_to_array(recipient);
    let existing_invoice_ids = decode_invoice_ids(
        client
            .list_invoices(recipient_bytes)
            .await
            .with_context(|| {
                format!(
                    "failed to load existing invoice ids for {} from storage canister",
                    recipient
                )
            })?,
    )?;

    let mut invoice_id = random_invoice_id(!args.batch, entry.chain_id);
    while existing_invoice_ids.contains(&invoice_id) {
        // regenerate if collision
        invoice_id = random_invoice_id(!args.batch, entry.chain_id);
    }
    println!("Recipient address  : {}", recipient);
    println!("Recipient chain ID : {}", recipient_chain_id);
    println!(
        "Invoice mode       : {}",
        if args.batch { "batch" } else { "single" }
    );
    println!(
        "Invoice ID         : 0x{}",
        hex::encode(invoice_id.as_slice())
    );

    let signing_key = signing_key_from_b256(private_key)?;
    let signature = sign_invoice(&signing_key, invoice_id)?;
    let submission = InvoiceSubmission {
        invoice_id: invoice_id.as_slice().to_vec(),
        signature: signature.to_vec(),
    };
    client
        .submit_invoice(&submission)
        .await
        .with_context(|| format!("failed to submit invoice for {}", entry.label))?;

    if args.batch {
        println!("Generated burn addresses:");
        for sub_id in 0..NUM_BATCH_INVOICES {
            let secret_and_tweak = SecretAndTweak::batch_invoice(
                invoice_id,
                sub_id as u32,
                seed,
                recipient_chain_id,
                recipient,
            );
            let burn_payload =
                FullBurnAddress::new(recipient_chain_id, recipient, &secret_and_tweak)?;
            let burn_address = burn_payload.burn_address().with_context(|| {
                format!(
                    "failed to derive burn address for issued batch invoice sub {}",
                    sub_id
                )
            })?;
            println!("- sub {:<2}: {}", sub_id, burn_address);
        }
    } else {
        let secret_and_tweak =
            SecretAndTweak::single_invoice(invoice_id, seed, recipient_chain_id, recipient);
        let burn_payload = FullBurnAddress::new(recipient_chain_id, recipient, &secret_and_tweak)?;
        let burn_address = burn_payload
            .burn_address()
            .context("failed to derive burn address for issued single invoice")?;
        println!("Burn address        : {}", burn_address);
    }

    Ok(())
}

pub async fn receive(
    common_args: &CommonArgs,
    args: &InvoiceReceiveArgs,
    token_entries: &[TokenEntry],
    hub: Option<&HubEntry>,
    private_key: B256,
) -> Result<()> {
    let Some(context) = build_invoice_receive_context(
        common_args,
        args,
        token_entries,
        hub,
        private_key,
        "invoice receive command",
    )
    .await?
    else {
        return Ok(());
    };

    for (chain_id, events) in &context.separated_events {
        print_events(*chain_id, events);
    }

    let InvoiceReceiveContext {
        verifier,
        indexer,
        gr,
        aggregation_tree_state,
        separated_events,
        burn_address_to_secret_and_tweak,
    } = context;

    let artifacts_dir = common_args
        .nova_artifacts_dir
        .as_deref()
        .ok_or(anyhow::anyhow!(
            "Nova artifacts directory must be specified for invoice receive command"
        ))?;

    let burn_address_to_secret = burn_address_to_secret_and_tweak
        .iter()
        .map(|(address, secret_and_tweak)| (*address, secret_and_tweak.secret))
        .collect::<HashMap<_, _>>();

    match redeem_transfers(
        common_args,
        &verifier,
        &indexer,
        &aggregation_tree_state,
        &separated_events,
        &burn_address_to_secret,
        gr,
        token_entries,
        private_key,
        artifacts_dir,
    )
    .await?
    {
        RedeemResult::AlreadyClaimed => {
            println!("No new eligible transfers found for the provided invoice ID.");
        }
        RedeemResult::NoProofs | RedeemResult::Submitted => {}
    }
    Ok(())
}

pub async fn status(
    common_args: &CommonArgs,
    args: &InvoiceReceiveArgs,
    token_entries: &[TokenEntry],
    hub: Option<&HubEntry>,
    private_key: B256,
) -> Result<()> {
    let Some(context) = build_invoice_receive_context(
        common_args,
        args,
        token_entries,
        hub,
        private_key,
        "invoice status command",
    )
    .await?
    else {
        return Ok(());
    };
    for burn_address in context.burn_address_to_secret_and_tweak.keys() {
        println!("Burn address: {}", burn_address);
    }
    for (chain_id, events) in &context.separated_events {
        print_events(*chain_id, events);
    }
    Ok(())
}

fn address_to_array(address: Address) -> [u8; 20] {
    let mut bytes = [0u8; 20];
    bytes.copy_from_slice(address.as_slice());
    bytes
}

fn decode_invoice_ids(raw: Vec<Vec<u8>>) -> Result<Vec<B256>> {
    raw.into_iter()
        .map(|bytes| decode_invoice_id(&bytes))
        .collect()
}

fn decode_invoice_id(bytes: &[u8]) -> Result<B256> {
    ensure!(
        bytes.len() == 32,
        "invoice identifier must be 32 bytes, got {}",
        bytes.len()
    );
    Ok(B256::from_slice(bytes))
}

fn signing_key_from_b256(secret: B256) -> Result<SigningKey> {
    let mut raw = [0u8; 32];
    raw.copy_from_slice(secret.as_slice());
    let field_bytes: FieldBytes = raw.into();
    SigningKey::from_bytes(&field_bytes)
        .map_err(|_| anyhow::anyhow!("failed to derive signing key from PRIVATE_KEY"))
}

fn sign_invoice(signing_key: &SigningKey, invoice_id: B256) -> Result<[u8; 65]> {
    let mut invoice_bytes = [0u8; 32];
    invoice_bytes.copy_from_slice(invoice_id.as_slice());
    let message = invoice_signature_message(&invoice_bytes);
    let digest: [u8; 32] = keccak256(&message).into();

    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(&digest)
        .map_err(|err| anyhow::anyhow!("failed to sign invoice submission: {err}"))?;

    let mut bytes = [0u8; 65];
    bytes[..64].copy_from_slice(&signature.to_bytes());
    bytes[64] = recovery_id.to_byte().saturating_add(27);
    Ok(bytes)
}
