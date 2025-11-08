use crate::{
    tokens::{JsHubEntry, JsTokenEntry},
    utils::{
        anyhow_to_js_error, format_u256_hex, fr_to_hex, hex_to_fr, parse_address_hex,
        parse_u256_hex, serde_error_to_js,
    },
};
use alloy::primitives::Address;
use anyhow::{Context, Result};
use api_types::indexer::IndexedEvent;
use client_common::{
    contracts::{hub::HubContract, verifier::VerifierContract, z_erc20::ZErc20Contract},
    indexer::HttpIndexerClient,
    teleport::{
        aggregation_tree::{self, AggregationTreeState},
        events::{self, EventsWithEligibility},
        merkle_proofs::{self, GlobalTeleportMerkleProof, LocalTeleportMerkleProof},
    },
    tokens::{HubEntry, TokenEntry},
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use zkp::{
    nova::constants::{AGGREGATION_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
    utils::{
        convertion::u256_to_fr,
        tree::merkle_tree::{MerkleProof, MerkleTree},
    },
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsAggregationTreeState {
    pub latest_agg_seq: u64,
    pub aggregation_root: String,
    pub snapshot: Vec<String>,
    pub transfer_tree_indices: Vec<u64>,
    pub chain_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsChainEvents {
    chain_id: u64,
    events: Vec<IndexedEvent>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsSeparatedChainEvents {
    chain_id: u64,
    events: EventsWithEligibility,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsLocalTeleportProof {
    tree_index: u64,
    event: IndexedEvent,
    siblings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsChainLocalProofs {
    chain_id: u64,
    proofs: Vec<JsLocalTeleportProof>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsGlobalTeleportProof {
    event: IndexedEvent,
    siblings: Vec<String>,
    leaf_index: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchAggregationTreeParams {
    #[serde(default)]
    event_block_span: Option<u64>,
    hub: JsHubEntry,
    token: JsTokenEntry,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchTransferEventsParams {
    indexer_url: String,
    #[serde(default)]
    indexer_fetch_limit: Option<usize>,
    tokens: Vec<JsTokenEntry>,
    burn_addresses: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeparateEventsParams {
    aggregation_state: JsAggregationTreeState,
    events: Vec<JsChainEvents>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchLocalTeleportProofsParams {
    indexer_url: String,
    token: JsTokenEntry,
    tree_index: u64,
    events: Vec<IndexedEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateGlobalTeleportProofsParams {
    aggregation_state: JsAggregationTreeState,
    proofs: Vec<JsChainLocalProofs>,
}

impl From<AggregationTreeState> for JsAggregationTreeState {
    fn from(state: AggregationTreeState) -> Self {
        JsAggregationTreeState {
            latest_agg_seq: state.latest_agg_seq,
            aggregation_root: format_u256_hex(state.aggregation_root),
            snapshot: state.snapshot.into_iter().map(format_u256_hex).collect(),
            transfer_tree_indices: state.tree_root_indices,
            chain_ids: state.chain_ids,
        }
    }
}

impl TryFrom<JsAggregationTreeState> for AggregationTreeState {
    type Error = anyhow::Error;

    fn try_from(value: JsAggregationTreeState) -> Result<Self> {
        let aggregation_root = parse_u256_hex(&value.aggregation_root)?;
        let snapshot = value
            .snapshot
            .iter()
            .map(|hex| parse_u256_hex(hex))
            .collect::<Result<Vec<_>, _>>()?;
        let mut tree = MerkleTree::new(AGGREGATION_TREE_HEIGHT);
        for (idx, entry) in snapshot.iter().enumerate() {
            if entry.is_zero() {
                continue;
            }
            tree.update_leaf(idx as u64, u256_to_fr(*entry));
        }
        if tree.get_root() != u256_to_fr(aggregation_root) {
            anyhow::bail!("aggregation snapshot root mismatch");
        }
        Ok(AggregationTreeState {
            latest_agg_seq: value.latest_agg_seq,
            aggregation_root,
            aggregation_tree: tree,
            tree_root_indices: value.transfer_tree_indices,
            chain_ids: value.chain_ids,
            snapshot,
        })
    }
}

#[wasm_bindgen]
pub async fn fetch_aggregation_tree_state(params: JsValue) -> Result<JsValue, JsValue> {
    console_error_panic_hook::set_once();
    let params: FetchAggregationTreeParams = serde_wasm_bindgen::from_value(params)
        .map_err(|err| JsValue::from_str(&err.to_string()))?;
    let state = fetch_aggregation_tree_state_impl(params)
        .await
        .map_err(anyhow_to_js_error)?;
    serde_wasm_bindgen::to_value(&state).map_err(serde_error_to_js)
}

#[wasm_bindgen]
pub async fn fetch_transfer_events(params: JsValue) -> Result<JsValue, JsValue> {
    console_error_panic_hook::set_once();
    let params: FetchTransferEventsParams = serde_wasm_bindgen::from_value(params)
        .map_err(|err| JsValue::from_str(&err.to_string()))?;
    let events = fetch_transfer_events_impl(params)
        .await
        .map_err(anyhow_to_js_error)?;
    serde_wasm_bindgen::to_value(&events).map_err(serde_error_to_js)
}

#[wasm_bindgen]
pub fn separate_events_by_eligibility(params: JsValue) -> Result<JsValue, JsValue> {
    console_error_panic_hook::set_once();
    let params: SeparateEventsParams = serde_wasm_bindgen::from_value(params)
        .map_err(|err| JsValue::from_str(&err.to_string()))?;
    let separated = separate_events_by_eligibility_impl(params).map_err(anyhow_to_js_error)?;
    serde_wasm_bindgen::to_value(&separated).map_err(serde_error_to_js)
}

#[wasm_bindgen]
pub async fn fetch_local_teleport_merkle_proofs(params: JsValue) -> Result<JsValue, JsValue> {
    console_error_panic_hook::set_once();
    let params: FetchLocalTeleportProofsParams = serde_wasm_bindgen::from_value(params)
        .map_err(|err| JsValue::from_str(&err.to_string()))?;
    let proofs = fetch_local_teleport_merkle_proofs_impl(params)
        .await
        .map_err(anyhow_to_js_error)?;
    serde_wasm_bindgen::to_value(&proofs).map_err(serde_error_to_js)
}

#[wasm_bindgen]
pub fn generate_global_teleport_merkle_proofs(params: JsValue) -> Result<JsValue, JsValue> {
    console_error_panic_hook::set_once();
    let params: GenerateGlobalTeleportProofsParams = serde_wasm_bindgen::from_value(params)
        .map_err(|err| JsValue::from_str(&err.to_string()))?;
    let proofs = generate_global_teleport_merkle_proofs_impl(params).map_err(anyhow_to_js_error)?;
    serde_wasm_bindgen::to_value(&proofs).map_err(serde_error_to_js)
}

async fn fetch_aggregation_tree_state_impl(
    params: FetchAggregationTreeParams,
) -> Result<JsAggregationTreeState> {
    let mut hub_entry: HubEntry = params.hub.try_into()?;
    hub_entry.normalize()?;
    let mut token_entry: TokenEntry = params.token.try_into()?;
    token_entry.normalize()?;
    let hub = build_hub(&hub_entry)?;
    let verifier = build_verifier(&token_entry)?;
    let span = params.event_block_span.unwrap_or(5_000);
    let state = aggregation_tree::fetch_aggregation_tree_state(span, &verifier, &hub).await?;
    Ok(JsAggregationTreeState::from(state))
}

async fn fetch_transfer_events_impl(
    params: FetchTransferEventsParams,
) -> Result<Vec<JsChainEvents>> {
    let token_entries = build_token_entries(params.tokens)?;
    let token_clients = build_token_clients(&token_entries)?;
    let indexer = build_indexer_client(&params.indexer_url)?;
    let burn_addresses = parse_addresses(&params.burn_addresses)?;
    let events = events::fetch_transfer_events(
        &indexer,
        params.indexer_fetch_limit,
        &token_entries,
        &token_clients,
        &burn_addresses,
    )
    .await?;
    Ok(hashmap_to_chain_events(events))
}

fn separate_events_by_eligibility_impl(
    params: SeparateEventsParams,
) -> Result<Vec<JsSeparatedChainEvents>> {
    let aggregation_state: AggregationTreeState = params.aggregation_state.try_into()?;
    let events_map = chain_events_to_hashmap(params.events);
    let separated = events::separate_events_by_eligibility(&aggregation_state, &events_map)?;
    Ok(separated
        .into_iter()
        .map(|(chain_id, events)| JsSeparatedChainEvents { chain_id, events })
        .collect())
}

async fn fetch_local_teleport_merkle_proofs_impl(
    params: FetchLocalTeleportProofsParams,
) -> Result<Vec<JsLocalTeleportProof>> {
    let mut token_entry: TokenEntry = params.token.try_into()?;
    token_entry.normalize()?;
    let indexer = build_indexer_client(&params.indexer_url)?;
    let proofs = merkle_proofs::fetch_local_teleport_merkle_proofs(
        &indexer,
        &token_entry,
        params.tree_index,
        &params.events,
    )
    .await?;
    Ok(proofs.into_iter().map(local_proof_to_js).collect())
}

fn generate_global_teleport_merkle_proofs_impl(
    params: GenerateGlobalTeleportProofsParams,
) -> Result<Vec<JsGlobalTeleportProof>> {
    let aggregation_state: AggregationTreeState = params.aggregation_state.try_into()?;
    let local_proofs = chain_local_proofs_to_map(params.proofs)?;
    let global =
        merkle_proofs::generate_global_teleport_merkle_proofs(&aggregation_state, &local_proofs)?;
    Ok(global.into_iter().map(global_proof_to_js).collect())
}

fn build_hub(entry: &HubEntry) -> Result<HubContract> {
    let provider = entry.provider()?;
    Ok(HubContract::new(provider, entry.hub_address))
}

fn build_verifier(entry: &TokenEntry) -> Result<VerifierContract> {
    let provider = entry.provider()?;
    Ok(VerifierContract::new(provider, entry.verifier_address).with_legacy_tx(entry.legacy_tx))
}

fn build_token_clients(entries: &[TokenEntry]) -> Result<Vec<ZErc20Contract>> {
    entries
        .iter()
        .map(|entry| {
            let provider = entry.provider()?;
            Ok(ZErc20Contract::new(provider, entry.token_address).with_legacy_tx(entry.legacy_tx))
        })
        .collect()
}

fn build_token_entries(raw: Vec<JsTokenEntry>) -> Result<Vec<TokenEntry>> {
    raw.into_iter()
        .map(|entry| {
            let mut converted: TokenEntry = entry.try_into()?;
            converted.normalize()?;
            Ok(converted)
        })
        .collect()
}

fn build_indexer_client(url: &str) -> Result<HttpIndexerClient> {
    let parsed = Url::parse(url).context("failed to parse indexer url")?;
    HttpIndexerClient::new(parsed).context("failed to construct indexer client")
}

fn parse_addresses(values: &[String]) -> Result<Vec<Address>> {
    values
        .iter()
        .map(|value| parse_address_hex(value))
        .collect()
}

fn hashmap_to_chain_events(mut events: HashMap<u64, Vec<IndexedEvent>>) -> Vec<JsChainEvents> {
    let mut chains: Vec<_> = events
        .drain()
        .map(|(chain_id, events)| JsChainEvents { chain_id, events })
        .collect();
    chains.sort_by_key(|entry| entry.chain_id);
    chains
}

fn chain_events_to_hashmap(entries: Vec<JsChainEvents>) -> HashMap<u64, Vec<IndexedEvent>> {
    let mut map = HashMap::new();
    for entry in entries {
        map.insert(entry.chain_id, entry.events);
    }
    map
}

fn local_proof_to_js(proof: LocalTeleportMerkleProof) -> JsLocalTeleportProof {
    let siblings = proof
        .local_merkle_proof
        .siblings
        .iter()
        .map(fr_to_hex)
        .collect();
    JsLocalTeleportProof {
        tree_index: proof.tree_index,
        event: proof.event,
        siblings,
    }
}

fn global_proof_to_js(proof: GlobalTeleportMerkleProof) -> JsGlobalTeleportProof {
    let siblings = proof
        .global_merkle_proof
        .siblings
        .iter()
        .map(fr_to_hex)
        .collect();
    JsGlobalTeleportProof {
        event: proof.event,
        siblings,
        leaf_index: proof.global_leaf_index,
    }
}

fn chain_local_proofs_to_map(
    entries: Vec<JsChainLocalProofs>,
) -> Result<HashMap<u64, Vec<LocalTeleportMerkleProof>>> {
    let mut map = HashMap::new();
    for entry in entries {
        let proofs = entry
            .proofs
            .into_iter()
            .map(|proof| proof.try_into())
            .collect::<Result<Vec<_>>>()?;
        map.insert(entry.chain_id, proofs);
    }
    Ok(map)
}

impl TryFrom<JsLocalTeleportProof> for LocalTeleportMerkleProof {
    type Error = anyhow::Error;

    fn try_from(value: JsLocalTeleportProof) -> Result<Self> {
        let siblings = value
            .siblings
            .iter()
            .map(|hex| hex_to_fr(hex))
            .collect::<Result<Vec<_>, _>>()?;
        if siblings.len() != TRANSFER_TREE_HEIGHT {
            anyhow::bail!(
                "expected {TRANSFER_TREE_HEIGHT} siblings, received {}",
                siblings.len()
            );
        }
        Ok(LocalTeleportMerkleProof {
            tree_index: value.tree_index,
            event: value.event,
            local_merkle_proof: MerkleProof { siblings },
        })
    }
}
