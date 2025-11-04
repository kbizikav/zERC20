use std::{
    convert::TryFrom,
    sync::Arc,
    time::{Duration, Instant},
};

use alloy::{
    network::Ethereum,
    primitives::{Address, B256, U256},
    providers::PendingTransactionBuilder,
};
use anyhow::{Context, Result, anyhow, bail};
use api_types::prover::CircuitKind;
use ark_bn254::{Fr, G1Projective as G1};
use ark_ff::{BigInteger, PrimeField, Zero};
use ark_grumpkin::Projective as G2;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use folding_schemes::{FoldingScheme, folding::nova::IVCProof};
use log::{debug, error, info, warn};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use sqlx::{PgPool, Row};
use tokio::time::sleep;

use crate::{
    config::RootJobConfig,
    jobs::try_acquire_lock,
    trees::{DbIncrementalMerkleTree, DbMerkleTreeConfig, HistoricalProof},
};
use client_common::{
    contracts::{
        utils::{get_provider, get_provider_with_fallback},
        verifier::VerifierContract,
        z_erc20::ZErc20Contract,
    },
    prover::{DeciderClient, HttpDeciderClient},
    tokens::{TokenEntry, TokenMetadata},
};
use zkp::{
    nova::{
        constants::TRANSFER_TREE_HEIGHT,
        params::{N, NovaParams},
        root_nova::{RootCircuit, RootExternalInputs},
    },
    utils::{
        convertion::{address_to_fr, fr_to_u256, u256_to_fr},
        poseidon::utils::circom_poseidon_config,
        tree::gadgets::{hash_chain::hash_chain, leaf_hash::compute_leaf_hash},
    },
};

const ROOT_LOCK_SALT: u64 = 0x524f4f54; // "ROOT"

type RootNovaInstance = N<RootCircuit<Fr>>;
type RootIvcProof = IVCProof<G1, G2>;

pub struct RootProverJob {
    pool: PgPool,
    tokens: Vec<RootTokenContext>,
    tree_config: DbMerkleTreeConfig,
    tree_height: u32,
    history_window: u64,
    compile_interval: Duration,
    submit_interval: Duration,
    nova_params: Arc<NovaParams<RootCircuit<Fr>>>,
    prover: Arc<dyn DeciderClient>,
    submitter_private_key: B256,
    prover_timeout: Duration,
    prover_poll_interval: Duration,
    submit_enabled: bool,
}

impl RootProverJob {
    pub async fn run_forever(&self) -> Result<()> {
        let mut last_compile = Instant::now() - self.compile_interval;
        let mut last_submit = Instant::now() - self.submit_interval;
        let min_interval = self.compile_interval.min(self.submit_interval);

        loop {
            let now = Instant::now();
            let should_compile = now.duration_since(last_compile) >= self.compile_interval;
            let should_submit = now.duration_since(last_submit) >= self.submit_interval;

            if should_compile || should_submit {
                self.run_cycle(should_compile, should_submit).await;
                let completed = Instant::now();
                if should_compile {
                    last_compile = completed;
                }
                if should_submit {
                    last_submit = completed;
                }
            }

            sleep(min_interval).await;
        }
    }

    pub async fn run_once(&self) -> Result<()> {
        self.run_cycle(true, true).await;
        Ok(())
    }

    async fn run_cycle(&self, do_compile: bool, do_submit: bool) {
        for token in &self.tokens {
            if let Err(err) = self.process_token(token, do_compile, do_submit).await {
                error!(
                    "root prover job failed for token '{}': {err:?}",
                    token.label
                );
            }
        }
    }

    async fn process_token(
        &self,
        token: &RootTokenContext,
        do_compile: bool,
        do_submit: bool,
    ) -> Result<()> {
        let Some(lease) = try_acquire_lock(&self.pool, token.lock_key).await? else {
            debug!(
                "skip root prover for '{}' due to lock contention",
                token.label
            );
            return Ok(());
        };

        let outcome = self.process_token_inner(token, do_compile, do_submit).await;

        if let Err(err) = lease.release().await {
            warn!(
                "failed to release root prover lease for '{}': {err:?}",
                token.label
            );
        }

        outcome
    }

    async fn process_token_inner(
        &self,
        token: &RootTokenContext,
        do_compile: bool,
        do_submit: bool,
    ) -> Result<()> {
        if !do_compile && !do_submit {
            return Ok(());
        }

        let token_id = match lookup_token_id(
            &self.pool,
            token.metadata.token_address,
            token.metadata.chain_id,
        )
        .await?
        {
            Some(id) => id,
            None => {
                debug!(
                    "token '{}' not yet registered in database; waiting for event sync",
                    token.label
                );
                return Ok(());
            }
        };

        let tree = DbIncrementalMerkleTree::new(
            self.pool.clone(),
            token_id,
            self.tree_height,
            self.tree_config.clone(),
        )
        .await
        .with_context(|| format!("failed to initialise merkle tree for '{}'", token.label))?;

        let current_index = token
            .token_contract
            .index()
            .await
            .with_context(|| format!("failed to query token index for '{}'", token.label))?;
        let latest_proved_index = if self.submit_enabled {
            token
                .verifier_contract
                .latest_proved_index()
                .await
                .with_context(|| format!("failed to query verifier state for '{}'", token.label))?
        } else {
            0
        };

        if latest_proved_index > current_index {
            warn!(
                "verifier latestProvedIndex ({}) ahead of token index ({}) for '{}'",
                latest_proved_index, current_index, token.label
            );
        }

        if latest_proved_index + self.history_window < current_index {
            bail!(
                "history window exhausted for '{}': latestProvedIndex={} currentIndex={} history_window={}. increase TREE_HISTORY_WINDOW / ROOT_HISTORY_WINDOW",
                token.label,
                latest_proved_index,
                current_index,
                self.history_window
            );
        }

        let mut state = ensure_state_alignment(&self.pool, token_id, latest_proved_index).await?;

        if do_compile {
            state = self
                .sync_ivc_proofs(token, token_id, &tree, state, current_index)
                .await?;
        }

        if do_submit {
            state = self
                .submit_if_ready(token, token_id, state, current_index)
                .await?;
        }

        let _ = &state;

        Ok(())
    }

    async fn sync_ivc_proofs(
        &self,
        token: &RootTokenContext,
        token_id: i64,
        tree: &DbIncrementalMerkleTree,
        mut state: RootProverState,
        contract_index: u64,
    ) -> Result<RootProverState> {
        let tree_index = tree
            .latest_index()
            .await
            .with_context(|| format!("failed to load latest tree index for '{}'", token.label))?;

        let target_index = tree_index.min(contract_index);
        if target_index <= state.last_compiled_index {
            debug!(
                "no new leaves to compile for '{}' (compiled={}, target={})",
                token.label, state.last_compiled_index, target_index
            );
            return Ok(state);
        }

        let events = fetch_event_batch(
            &self.pool,
            token_id,
            state.last_compiled_index,
            target_index,
        )
        .await
        .with_context(|| format!("failed to fetch events for '{}'", token.label))?;

        if events.is_empty() {
            debug!(
                "no event records found while attempting to compile '{}' (compiled={}, target={})",
                token.label, state.last_compiled_index, target_index
            );
            return Ok(state);
        }

        let mut nova = initialise_nova(
            &self.nova_params,
            token_id,
            &self.pool,
            tree,
            state.base_index,
            state.last_compiled_index,
        )
        .await
        .with_context(|| format!("failed to initialise nova for '{}'", token.label))?;

        let mut rng = ChaCha20Rng::from_entropy();
        let mut current_index = state.last_compiled_index;

        for event in events {
            let expected_index = current_index;
            if event.event_index != expected_index {
                warn!(
                    "encountered non-contiguous event for '{}': expected {}, got {}",
                    token.label, expected_index, event.event_index
                );
                break;
            }

            let proof = tree
                .prove(event.event_index + 1, event.event_index)
                .await
                .with_context(|| {
                    format!(
                        "failed to build merkle proof for '{}' at {}",
                        token.label, event.event_index
                    )
                })?;

            let external_inputs = to_external_inputs(event.address, event.value, &proof)?;
            nova.prove_step(&mut rng, external_inputs, None)
                .with_context(|| {
                    format!(
                        "failed to extend nova proof for '{}' at step {}",
                        token.label, event.event_index
                    )
                })?;

            current_index += 1;
            let state_snapshot = nova.state();
            let ivc_proof = nova.ivc_proof();
            if let Err(err) = self.nova_params.verify(ivc_proof.clone()) {
                let state_index = state_snapshot.get(0).copied().unwrap_or_else(Fr::zero);
                let state_hash_chain = state_snapshot.get(1).copied().unwrap_or_else(Fr::zero);
                let state_root = state_snapshot.get(2).copied().unwrap_or_else(Fr::zero);
                let db_hash_chain = tree.hash_chain_at(current_index).await.ok().flatten();
                let db_root = tree.root_at(current_index).await.ok().flatten();
                let previous_hash_chain = if current_index > 1 {
                    tree.hash_chain_at(current_index - 1)
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or_default()
                } else {
                    U256::ZERO
                };
                let expected_hash_chain =
                    hash_chain(previous_hash_chain, event.address, event.value);
                let expected_leaf =
                    compute_leaf_hash(address_to_fr(event.address), u256_to_fr(event.value));
                let expected_root = proof.proof.get_root(expected_leaf, proof.leaf_index);
                debug!(
                    "nova verify failed at index {} for '{}': address=0x{}, value={}, state_index={}, state_hash_chain=0x{}, state_root=0x{}, db_hash_chain={:?}, db_root={:?}, expected_hash_chain=0x{}, expected_root=0x{}",
                    current_index,
                    token.label,
                    hex::encode(event.address.as_slice()),
                    event.value,
                    fr_to_u256(state_index),
                    fr_to_u256(state_hash_chain),
                    fr_to_u256(state_root),
                    db_hash_chain.map(|v| format!("0x{}", hex::encode(v.to_be_bytes::<32>()))),
                    db_root.map(|v| format!("0x{}", hex::encode(fr_to_bytes(v)))),
                    hex::encode(expected_hash_chain.to_be_bytes::<32>()),
                    hex::encode(fr_to_bytes(expected_root))
                );
                return Err(err).context(format!(
                    "failed to verify IVC proof for '{}' at index {}",
                    token.label, current_index
                ));
            }
            let proof_bytes = serialize_ivc_proof(&ivc_proof)?;
            let state_hash_chain = state_snapshot
                .get(1)
                .copied()
                .ok_or_else(|| anyhow!("nova state missing hash chain component"))?;
            let state_root = state_snapshot
                .get(2)
                .copied()
                .ok_or_else(|| anyhow!("nova state missing root component"))?;

            upsert_ivc_proof(
                &self.pool,
                token_id,
                state.base_index,
                current_index,
                &proof_bytes,
                state_hash_chain,
                state_root,
            )
            .await
            .with_context(|| {
                format!(
                    "failed to persist IVC proof for '{}' at index {}",
                    token.label, current_index
                )
            })?;
        }

        if current_index > state.last_compiled_index {
            update_last_compiled_index(&self.pool, token_id, current_index).await?;
            state.last_compiled_index = current_index;
            info!(
                "compiled root IVC proofs for '{}' up to index {}",
                token.label, current_index
            );
        }

        Ok(state)
    }

    async fn submit_if_ready(
        &self,
        token: &RootTokenContext,
        token_id: i64,
        mut state: RootProverState,
        contract_index: u64,
    ) -> Result<RootProverState> {
        if state.last_compiled_index <= state.last_submitted_index {
            debug!(
                "no compiled proofs ready for submission for '{}' (compiled={}, submitted={})",
                token.label, state.last_compiled_index, state.last_submitted_index
            );
            return Ok(state);
        }

        if state.last_compiled_index > contract_index {
            debug!(
                "compiled proofs for '{}' ahead of on-chain index (compiled={}, contract={})",
                token.label, state.last_compiled_index, contract_index
            );
            return Ok(state);
        }

        let target_index = state.last_compiled_index;
        let proof_record = wait_for_ivc_record(
            &self.pool,
            token_id,
            target_index,
            self.prover_timeout,
            self.prover_poll_interval,
        )
        .await?;

        let ivc_bytes = if target_index - state.base_index == 1 {
            // apply dummy step
            let mut nova = load_nova_from_ivc(&self.nova_params, &proof_record.ivc_proof)?;
            let mut rng = ChaCha20Rng::from_entropy();
            let dummy = RootExternalInputs::<Fr> {
                is_dummy: true,
                address: Fr::zero(),
                value: Fr::zero(),
                siblings: [Fr::zero(); TRANSFER_TREE_HEIGHT],
            };
            nova.prove_step(&mut rng, dummy, None)
                .context("failed to append dummy step before submission")?;
            serialize_ivc_proof(&nova.ivc_proof())?
        } else {
            proof_record.ivc_proof.clone()
        };

        if !self.submit_enabled {
            self.prover
                .produce_decider_proof(CircuitKind::Root, ivc_bytes.as_slice())
                .await
                .context("root prover decider generation failed")?;
            return Ok(state);
        }

        let pendingreservation = if state.pending_reserved_index == Some(target_index) {
            state
                .pending_reserved_hash_chain
                .map(|hash| (target_index, hash))
        } else {
            None
        };

        let (reserved_index, reserved_hash_chain) = match pendingreservation {
            Some(res) => res,
            None => {
                let (idx, hash_chain) =
                    self.reserve_hash_chain(token).await.with_context(|| {
                        format!("failed to reserve hash chain for '{}'", token.label)
                    })?;
                persist_pending_reservation(&self.pool, token_id, idx, &hash_chain).await?;
                state.pending_reserved_index = Some(idx);
                state.pending_reserved_hash_chain = Some(hash_chain);
                (idx, hash_chain)
            }
        };

        if reserved_index != target_index {
            warn!(
                "reserved index {} for '{}' does not match target {} – waiting for proofs to catch up",
                reserved_index, token.label, target_index
            );
            return Ok(state);
        }

        if fr_to_u256(proof_record.state_hash_chain) != reserved_hash_chain {
            warn!(
                "reserved hash chain mismatch for '{}': expected {}, proof {}",
                token.label,
                reserved_hash_chain,
                fr_to_u256(proof_record.state_hash_chain)
            );
            return Ok(state);
        }

        let decider = self
            .prover
            .produce_decider_proof(CircuitKind::Root, ivc_bytes.as_slice())
            .await
            .context("root prover decider generation failed")?;

        let receipt = self
            .submit_transfer_root(token, &decider)
            .await
            .with_context(|| format!("failed to submit proveTransferRoot for '{}'", token.label))?;

        info!(
            "submitted transfer root for '{}' at index {} (tx={:?})",
            token.label, target_index, receipt.transaction_hash
        );

        // Reset state with new base index
        reset_state_after_submission(&self.pool, token_id, target_index).await?;
        state.base_index = target_index;
        state.last_compiled_index = target_index;
        state.last_submitted_index = target_index;
        state.pending_reserved_index = None;
        state.pending_reserved_hash_chain = None;

        // Purge compiled proofs – they are no longer valid for the new base
        delete_ivc_proofs(&self.pool, token_id).await?;

        // Rebuild base snapshot to align with new state by storing zero-step proof if needed
        ensure_state_alignment(&self.pool, token_id, target_index).await?;

        Ok(state)
    }

    async fn reserve_hash_chain(&self, token: &RootTokenContext) -> Result<(u64, U256)> {
        let pending = token
            .verifier_contract
            .reserve_hash_chain(self.submitter_private_key)
            .await?;
        let receipt = wait_for_receipt(pending).await?;
        token
            .verifier_contract
            .parse_hash_chain_reserved(&receipt)
            .context("failed to parse HashChainReserved event")
    }

    async fn submit_transfer_root(
        &self,
        token: &RootTokenContext,
        decider_proof: &[u8],
    ) -> Result<alloy::rpc::types::TransactionReceipt> {
        let pending = token
            .verifier_contract
            .prove_transfer_root(self.submitter_private_key, decider_proof)
            .await?;
        wait_for_receipt(pending).await
    }
}

pub struct RootProverJobBuilder {
    pool: PgPool,
    root_config: RootJobConfig,
    tree_config: DbMerkleTreeConfig,
    tree_height: u32,
    tokens: Vec<TokenEntry>,
    prover_override: Option<Arc<dyn DeciderClient>>,
    submission_enabled: bool,
}

impl RootProverJobBuilder {
    pub fn new(
        pool: PgPool,
        root_config: RootJobConfig,
        tree_config: DbMerkleTreeConfig,
        tree_height: u32,
        tokens: Vec<TokenEntry>,
    ) -> Self {
        Self {
            pool,
            root_config,
            tree_config,
            tree_height,
            tokens,
            prover_override: None,
            submission_enabled: true,
        }
    }

    pub fn with_prover(mut self, prover: Arc<dyn DeciderClient>) -> Self {
        self.prover_override = Some(prover);
        self
    }

    pub fn with_submission_enabled(mut self, enabled: bool) -> Self {
        self.submission_enabled = enabled;
        self
    }

    pub fn into_job(self) -> Result<RootProverJob> {
        let mut contexts = Vec::with_capacity(self.tokens.len());
        for token in self.tokens {
            let provider = if token.rpc_urls.len() == 1 {
                get_provider(token.rpc_urls.first().expect("rpc urls not empty")).with_context(
                    || format!("failed to build provider for token '{}'", token.label),
                )?
            } else {
                get_provider_with_fallback(&token.rpc_urls).with_context(|| {
                    format!(
                        "failed to build fallback provider for token '{}'",
                        token.label
                    )
                })?
            };

            let token_contract = ZErc20Contract::new(provider.clone(), token.token_address)
                .with_legacy_tx(token.legacy_tx);
            let verifier_contract = VerifierContract::new(provider.clone(), token.verifier_address)
                .with_legacy_tx(token.legacy_tx);

            let metadata = token.metadata();
            let lock_key = token.lock_key_with_salt(ROOT_LOCK_SALT);
            contexts.push(RootTokenContext {
                label: token.label.clone(),
                metadata,
                token_contract,
                verifier_contract,
                lock_key,
            });
        }

        let nova_params = Arc::new(load_root_nova_params(&self.root_config.artifacts_dir)?);
        let prover: Arc<dyn DeciderClient> = match self.prover_override {
            Some(custom) => custom,
            None => Arc::new(HttpDeciderClient::new(
                self.root_config.prover_url.clone(),
                self.root_config.prover_poll_interval,
                self.root_config.prover_timeout,
            )?),
        };

        let compile_interval = self.root_config.interval();
        let submit_interval = self.root_config.submit_interval();

        Ok(RootProverJob {
            pool: self.pool,
            tokens: contexts,
            tree_config: self.tree_config,
            tree_height: self.tree_height,
            history_window: self.root_config.history_window,
            compile_interval,
            submit_interval,
            nova_params,
            prover,
            submitter_private_key: self.root_config.submitter_private_key,
            prover_timeout: self.root_config.prover_timeout,
            prover_poll_interval: self.root_config.prover_poll_interval,
            submit_enabled: self.submission_enabled,
        })
    }
}

struct RootTokenContext {
    label: String,
    metadata: TokenMetadata,
    token_contract: ZErc20Contract,
    verifier_contract: VerifierContract,
    lock_key: i64,
}

struct RootProverState {
    base_index: u64,
    last_compiled_index: u64,
    last_submitted_index: u64,
    pending_reserved_index: Option<u64>,
    pending_reserved_hash_chain: Option<U256>,
}

struct EventRow {
    event_index: u64,
    address: Address,
    value: U256,
}

struct IvcRecord {
    ivc_proof: Vec<u8>,
    state_hash_chain: Fr,
    state_root: Fr,
}

async fn lookup_token_id(pool: &PgPool, address: Address, chain_id: u64) -> Result<Option<i64>> {
    let chain_id_i64 = i64::try_from(chain_id)
        .with_context(|| format!("chain id {} exceeds i64 range", chain_id))?;
    let row = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM tokens
        WHERE token_address = $1 AND chain_id = $2
        "#,
    )
    .bind(address.as_slice())
    .bind(chain_id_i64)
    .fetch_optional(pool)
    .await
    .context("failed to lookup token id")?;
    Ok(row)
}

async fn ensure_state_alignment(
    pool: &PgPool,
    token_id: i64,
    base_index: u64,
) -> Result<RootProverState> {
    let existing = load_prover_state(pool, token_id).await?;

    match existing {
        Some(state) if state.base_index == base_index => Ok(state),
        _ => {
            delete_ivc_proofs(pool, token_id).await?;
            upsert_prover_state(
                pool, token_id, base_index, base_index, base_index, None, None,
            )
            .await?;
            Ok(RootProverState {
                base_index,
                last_compiled_index: base_index,
                last_submitted_index: base_index,
                pending_reserved_index: None,
                pending_reserved_hash_chain: None,
            })
        }
    }
}

async fn load_prover_state(pool: &PgPool, token_id: i64) -> Result<Option<RootProverState>> {
    let row = sqlx::query(
        r#"
        SELECT base_index,
               last_compiled_index,
               last_submitted_index,
               pending_reserved_index,
               pending_reserved_hash_chain
        FROM root_prover_state
        WHERE token_id = $1
        "#,
    )
    .bind(token_id)
    .fetch_optional(pool)
    .await
    .context("failed to load root prover state")?;

    Ok(row.map(|record| {
        let base_index: i64 = record.get("base_index");
        let last_compiled_index: i64 = record.get("last_compiled_index");
        let last_submitted_index: i64 = record.get("last_submitted_index");
        let pending_reserved_index: Option<i64> = record.get("pending_reserved_index");
        let pending_reserved_hash_chain: Option<Vec<u8>> =
            record.get("pending_reserved_hash_chain");

        RootProverState {
            base_index: base_index as u64,
            last_compiled_index: last_compiled_index as u64,
            last_submitted_index: last_submitted_index as u64,
            pending_reserved_index: pending_reserved_index.map(|v| v as u64),
            pending_reserved_hash_chain: pending_reserved_hash_chain
                .map(|bytes| U256::from_be_slice(bytes.as_slice())),
        }
    }))
}

async fn upsert_prover_state(
    pool: &PgPool,
    token_id: i64,
    base_index: u64,
    last_compiled_index: u64,
    last_submitted_index: u64,
    pending_reserved_index: Option<u64>,
    pending_reserved_hash_chain: Option<&U256>,
) -> Result<()> {
    let base_i64 = u64_to_i64("base_index", base_index)?;
    let compiled_i64 = u64_to_i64("last_compiled_index", last_compiled_index)?;
    let submitted_i64 = u64_to_i64("last_submitted_index", last_submitted_index)?;
    let pending_index_i64 = pending_reserved_index
        .map(|v| u64_to_i64("pending_reserved_index", v))
        .transpose()?;
    let hash_chain_bytes = pending_reserved_hash_chain.map(|v| v.to_be_bytes::<32>().to_vec());

    sqlx::query(
        r#"
        INSERT INTO root_prover_state (
            token_id,
            base_index,
            last_compiled_index,
            last_submitted_index,
            pending_reserved_index,
            pending_reserved_hash_chain,
            updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, NOW())
        ON CONFLICT (token_id)
        DO UPDATE SET
            base_index = EXCLUDED.base_index,
            last_compiled_index = EXCLUDED.last_compiled_index,
            last_submitted_index = EXCLUDED.last_submitted_index,
            pending_reserved_index = EXCLUDED.pending_reserved_index,
            pending_reserved_hash_chain = EXCLUDED.pending_reserved_hash_chain,
            updated_at = NOW()
        "#,
    )
    .bind(token_id)
    .bind(base_i64)
    .bind(compiled_i64)
    .bind(submitted_i64)
    .bind(pending_index_i64)
    .bind(hash_chain_bytes)
    .execute(pool)
    .await
    .context("failed to upsert root prover state")?;

    Ok(())
}

async fn update_last_compiled_index(pool: &PgPool, token_id: i64, index: u64) -> Result<()> {
    let index_i64 = u64_to_i64("last_compiled_index", index)?;
    sqlx::query(
        r#"
        UPDATE root_prover_state
        SET last_compiled_index = $2,
            updated_at = NOW()
        WHERE token_id = $1
        "#,
    )
    .bind(token_id)
    .bind(index_i64)
    .execute(pool)
    .await
    .context("failed to update last compiled index")?;
    Ok(())
}

async fn reset_state_after_submission(pool: &PgPool, token_id: i64, new_base: u64) -> Result<()> {
    upsert_prover_state(pool, token_id, new_base, new_base, new_base, None, None).await
}

async fn persist_pending_reservation(
    pool: &PgPool,
    token_id: i64,
    index: u64,
    hash_chain: &U256,
) -> Result<()> {
    let snapshot = load_prover_state(pool, token_id).await?;
    let base = snapshot.as_ref().map(|s| s.base_index).unwrap_or(index);
    let compiled = snapshot
        .as_ref()
        .map(|s| s.last_compiled_index)
        .unwrap_or(index);
    let submitted = snapshot
        .as_ref()
        .map(|s| s.last_submitted_index)
        .unwrap_or(index);
    upsert_prover_state(
        pool,
        token_id,
        base,
        compiled,
        submitted,
        Some(index),
        Some(hash_chain),
    )
    .await
}

async fn delete_ivc_proofs(pool: &PgPool, token_id: i64) -> Result<()> {
    sqlx::query(
        r#"
        DELETE FROM root_ivc_proofs
        WHERE token_id = $1
        "#,
    )
    .bind(token_id)
    .execute(pool)
    .await
    .context("failed to delete IVC proofs")?;
    Ok(())
}

async fn upsert_ivc_proof(
    pool: &PgPool,
    token_id: i64,
    start_index: u64,
    end_index: u64,
    proof_bytes: &[u8],
    state_hash_chain: Fr,
    state_root: Fr,
) -> Result<()> {
    let hash_chain_bytes = fr_to_bytes(state_hash_chain);
    let root_bytes = fr_to_bytes(state_root);
    let start_i64 = u64_to_i64("start_index", start_index)?;
    let end_i64 = u64_to_i64("end_index", end_index)?;

    sqlx::query(
        r#"
        INSERT INTO root_ivc_proofs (
            token_id,
            start_index,
            end_index,
            ivc_proof,
            state_index,
            state_hash_chain,
            state_root,
            created_at,
            updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), NOW())
        ON CONFLICT (token_id, end_index)
        DO UPDATE SET
            ivc_proof = EXCLUDED.ivc_proof,
            state_index = EXCLUDED.state_index,
            state_hash_chain = EXCLUDED.state_hash_chain,
            state_root = EXCLUDED.state_root,
            updated_at = NOW()
        "#,
    )
    .bind(token_id)
    .bind(start_i64)
    .bind(end_i64)
    .bind(proof_bytes)
    .bind(end_i64)
    .bind(hash_chain_bytes.as_slice())
    .bind(root_bytes.as_slice())
    .execute(pool)
    .await
    .context("failed to upsert root IVC proof")?;

    Ok(())
}

async fn fetch_event_batch(
    pool: &PgPool,
    token_id: i64,
    start_index: u64,
    end_index: u64,
) -> Result<Vec<EventRow>> {
    if end_index <= start_index {
        return Ok(Vec::new());
    }

    let start_i64 = u64_to_i64("event_index_start", start_index)?;
    let end_i64 = u64_to_i64("event_index_end", end_index - 1)?;

    let rows = sqlx::query(
        r#"
        SELECT event_index, to_address, value
        FROM indexed_transfer_events
        WHERE token_id = $1
          AND event_index >= $2
          AND event_index <= $3
        ORDER BY event_index ASC
        "#,
    )
    .bind(token_id)
    .bind(start_i64)
    .bind(end_i64)
    .fetch_all(pool)
    .await
    .context("failed to fetch event rows for root prover")?;

    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let index_i64: i64 = row.get("event_index");
        let address_bytes: Vec<u8> = row.get("to_address");
        let value_bytes: Vec<u8> = row.get("value");
        let index_u64 = index_i64 as u64;
        let address = parse_address(address_bytes.as_slice())?;
        let value = parse_u256(value_bytes.as_slice())?;
        events.push(EventRow {
            event_index: index_u64,
            address,
            value,
        });
    }

    Ok(events)
}

async fn wait_for_ivc_record(
    pool: &PgPool,
    token_id: i64,
    end_index: u64,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<IvcRecord> {
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() > deadline {
            bail!(
                "timed out waiting for ivc proof at index {} after {:?}",
                end_index,
                timeout
            );
        }

        if let Some(record) = load_ivc_record(pool, token_id, end_index).await? {
            return Ok(record);
        }

        sleep(poll_interval).await;
    }
}

async fn load_ivc_record(
    pool: &PgPool,
    token_id: i64,
    end_index: u64,
) -> Result<Option<IvcRecord>> {
    let end_i64 = u64_to_i64("end_index", end_index)?;
    let row = sqlx::query(
        r#"
        SELECT start_index,
               end_index,
               ivc_proof,
               state_hash_chain,
               state_root
        FROM root_ivc_proofs
        WHERE token_id = $1 AND end_index = $2
        "#,
    )
    .bind(token_id)
    .bind(end_i64)
    .fetch_optional(pool)
    .await
    .context("failed to load IVC proof record")?;

    Ok(row.map(|record| {
        let _start_index: i64 = record.get("start_index");
        let _end_index: i64 = record.get("end_index");
        let ivc_proof: Vec<u8> = record.get("ivc_proof");
        let state_hash_chain: Vec<u8> = record.get("state_hash_chain");
        let state_root: Vec<u8> = record.get("state_root");

        IvcRecord {
            ivc_proof,
            state_hash_chain: Fr::from_be_bytes_mod_order(state_hash_chain.as_slice()),
            state_root: Fr::from_be_bytes_mod_order(state_root.as_slice()),
        }
    }))
}

fn to_external_inputs(
    address: Address,
    value: U256,
    proof: &HistoricalProof,
) -> Result<RootExternalInputs<Fr>> {
    let siblings_vec = proof.proof.siblings.clone();
    if siblings_vec.len() != TRANSFER_TREE_HEIGHT {
        bail!(
            "unexpected sibling path length: expected {}, got {}",
            TRANSFER_TREE_HEIGHT,
            siblings_vec.len()
        );
    }
    let siblings_array: [Fr; TRANSFER_TREE_HEIGHT] = siblings_vec
        .try_into()
        .map_err(|_| anyhow!("failed to convert sibling path into array"))?;

    Ok(RootExternalInputs::<Fr> {
        is_dummy: false,
        address: address_to_fr(address),
        value: u256_to_fr(value),
        siblings: siblings_array,
    })
}

fn parse_address(bytes: &[u8]) -> Result<Address> {
    if bytes.len() != 20 {
        bail!("address bytes must be 20, got {}", bytes.len());
    }
    Ok(Address::from_slice(bytes))
}

fn parse_u256(bytes: &[u8]) -> Result<U256> {
    if bytes.len() != 32 {
        bail!("value bytes must be 32, got {}", bytes.len());
    }
    Ok(U256::from_be_slice(bytes))
}

async fn initialise_nova(
    nova_params: &Arc<NovaParams<RootCircuit<Fr>>>,
    token_id: i64,
    pool: &PgPool,
    tree: &DbIncrementalMerkleTree,
    base_index: u64,
    compiled_index: u64,
) -> Result<RootNovaInstance> {
    let z0 = load_state_vector(tree, base_index)
        .await
        .context("failed to load base state vector")?;

    if compiled_index == base_index {
        return nova_params
            .initial_nova(z0)
            .context("failed to initialise nova instance");
    }

    let ivc_record = load_ivc_record(pool, token_id, compiled_index)
        .await?
        .ok_or_else(|| anyhow!("missing IVC proof for compiled index {}", compiled_index))?;

    let ivc = deserialize_ivc_proof(&ivc_record.ivc_proof)?;
    let nova = nova_params
        .nova_from_ivc_proof(ivc)
        .context("failed to reconstruct nova from ivc proof")?;

    // ensure nova state matches stored root/hash chain
    let state = nova.state();
    if state.get(1).copied() != Some(ivc_record.state_hash_chain)
        || state.get(2).copied() != Some(ivc_record.state_root)
    {
        warn!(
            "reconstructed nova state mismatch for compiled index {}",
            compiled_index
        );
    }

    Ok(nova)
}

async fn load_state_vector(tree: &DbIncrementalMerkleTree, index: u64) -> Result<Vec<Fr>> {
    let hash_chain = tree
        .hash_chain_at(index)
        .await
        .context("failed to fetch hash chain for base index")?
        .unwrap_or(U256::ZERO);
    let root = tree
        .root_at(index)
        .await
        .context("failed to fetch root for base index")?
        .unwrap_or_else(|| tree.zero_root());

    Ok(vec![Fr::from(index), u256_to_fr(hash_chain), root])
}

fn load_root_nova_params(artifacts_dir: &std::path::Path) -> Result<NovaParams<RootCircuit<Fr>>> {
    let pp_path = artifacts_dir.join("root_nova_pp.bin");
    let vp_path = artifacts_dir.join("root_nova_vp.bin");
    let pp_bytes =
        std::fs::read(&pp_path).with_context(|| format!("failed to read {}", pp_path.display()))?;
    let vp_bytes =
        std::fs::read(&vp_path).with_context(|| format!("failed to read {}", vp_path.display()))?;
    let f_params = circom_poseidon_config::<Fr>();
    NovaParams::<RootCircuit<Fr>>::from_bytes(f_params, pp_bytes, vp_bytes)
        .context("failed to load root nova parameters")
}

fn serialize_ivc_proof(proof: &RootIvcProof) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    proof
        .serialize_uncompressed(&mut bytes)
        .context("failed to serialize ivc proof")?;
    Ok(bytes)
}

fn deserialize_ivc_proof(bytes: &[u8]) -> Result<RootIvcProof> {
    let mut cursor = std::io::Cursor::new(bytes);
    IVCProof::deserialize_uncompressed(&mut cursor).context("failed to deserialize ivc proof")
}

fn load_nova_from_ivc(
    nova_params: &Arc<NovaParams<RootCircuit<Fr>>>,
    ivc_bytes: &[u8],
) -> Result<RootNovaInstance> {
    let ivc = deserialize_ivc_proof(ivc_bytes)?;
    nova_params
        .nova_from_ivc_proof(ivc)
        .context("failed to load nova instance from ivc proof")
}

fn u64_to_i64(label: &str, value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| {
        anyhow!(
            "{label} exceeds i64 range: {value}",
            label = label,
            value = value
        )
    })
}

fn fr_to_bytes(value: Fr) -> [u8; 32] {
    let mut bytes = value.into_bigint().to_bytes_be();
    if bytes.len() < 32 {
        let mut padded = vec![0u8; 32 - bytes.len()];
        padded.extend_from_slice(&bytes);
        bytes = padded;
    }
    bytes
        .as_slice()
        .try_into()
        .expect("field element serialization to 32 bytes")
}

async fn wait_for_receipt(
    pending: PendingTransactionBuilder<Ethereum>,
) -> Result<alloy::rpc::types::TransactionReceipt> {
    let receipt = pending
        .get_receipt()
        .await
        .context("failed to fetch transaction receipt")?;
    if receipt.status() {
        Ok(receipt)
    } else {
        bail!("transaction reverted: {:?}", receipt);
    }
}
