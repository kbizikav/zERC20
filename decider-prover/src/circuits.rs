use std::{fs, io::Cursor, path::Path};

use api_types::prover::CircuitKind;
use ark_bn254::{Fr, G1Projective as G1};
use ark_grumpkin::Projective as G2;
use ark_serialize::CanonicalDeserialize;
use folding_schemes::{folding::nova::IVCProof, frontend::FCircuit};
use zkp::nova::{
    constants::{GLOBAL_TRANSFER_TREE_HEIGHT, TRANSFER_TREE_HEIGHT},
    params::{DeciderParams, FParams, NovaParams},
    root_nova::RootCircuit,
    withdraw_nova::WithdrawCircuit,
};
use zkp::utils::poseidon::utils::circom_poseidon_config;

use crate::{config::CircuitEnablement, errors::ProverError};

pub struct ProverEngine {
    root: Option<CircuitContext<RootCircuit<Fr>>>,
    withdraw_local: Option<CircuitContext<WithdrawCircuit<Fr, TRANSFER_TREE_HEIGHT>>>,
    withdraw_global: Option<CircuitContext<WithdrawCircuit<Fr, GLOBAL_TRANSFER_TREE_HEIGHT>>>,
}

impl ProverEngine {
    pub fn load(artifacts_dir: &Path, circuits: &CircuitEnablement) -> Result<Self, ProverError> {
        let poseidon = circom_poseidon_config::<Fr>();
        let root = if circuits.root() {
            Some(CircuitContext::load(
                "root",
                artifacts_dir,
                poseidon.clone(),
            )?)
        } else {
            None
        };
        let withdraw_local = if circuits.withdraw_local() {
            Some(CircuitContext::load(
                "withdraw_local",
                artifacts_dir,
                poseidon.clone(),
            )?)
        } else {
            None
        };
        let withdraw_global = if circuits.withdraw_global() {
            Some(CircuitContext::load(
                "withdraw_global",
                artifacts_dir,
                poseidon,
            )?)
        } else {
            None
        };

        Ok(Self {
            root,
            withdraw_local,
            withdraw_global,
        })
    }

    pub fn generate_decider_proof(
        &self,
        circuit: CircuitKind,
        ivc_proof_bytes: &[u8],
    ) -> Result<Vec<u8>, ProverError> {
        match circuit {
            CircuitKind::Root => self
                .root
                .as_ref()
                .ok_or_else(|| disabled_circuit_error(&circuit))?
                .generate(ivc_proof_bytes),
            CircuitKind::WithdrawLocal => self
                .withdraw_local
                .as_ref()
                .ok_or_else(|| disabled_circuit_error(&circuit))?
                .generate(ivc_proof_bytes),
            CircuitKind::WithdrawGlobal => self
                .withdraw_global
                .as_ref()
                .ok_or_else(|| disabled_circuit_error(&circuit))?
                .generate(ivc_proof_bytes),
        }
    }
}

fn disabled_circuit_error(circuit: &CircuitKind) -> ProverError {
    ProverError::InvalidInput(format!("{circuit} circuit is disabled in the prover"))
}

struct CircuitContext<C>
where
    C: FCircuit<Fr>,
    FParams<C>: Clone,
{
    nova: NovaParams<C>,
    decider: DeciderParams<C>,
}

impl<C> CircuitContext<C>
where
    C: FCircuit<Fr>,
    FParams<C>: Clone,
{
    fn load(prefix: &str, dir: &Path, f_params: FParams<C>) -> Result<Self, ProverError> {
        let nova_pp = read_artifact(dir, prefix, "nova_pp.bin")?;
        let nova_vp = read_artifact(dir, prefix, "nova_vp.bin")?;
        let decider_pp = read_artifact(dir, prefix, "decider_pp.bin")?;
        let decider_vp = read_artifact(dir, prefix, "decider_vp.bin")?;

        let nova = NovaParams::<C>::from_bytes(f_params.clone(), nova_pp, nova_vp)?;
        let decider = DeciderParams::<C>::from_bytes(decider_pp, decider_vp)?;

        Ok(Self { nova, decider })
    }

    fn generate(&self, ivc_proof_bytes: &[u8]) -> Result<Vec<u8>, ProverError> {
        produce_decider_proof(&self.nova, &self.decider, ivc_proof_bytes)
    }
}

fn produce_decider_proof<C>(
    nova_params: &NovaParams<C>,
    decider_params: &DeciderParams<C>,
    ivc_proof_bytes: &[u8],
) -> Result<Vec<u8>, ProverError>
where
    C: FCircuit<Fr>,
    FParams<C>: Clone,
{
    let mut reader = Cursor::new(ivc_proof_bytes);
    let ivc_proof: IVCProof<G1, G2> =
        IVCProof::deserialize_uncompressed(&mut reader).map_err(|err| {
            ProverError::InvalidInput(format!("failed to deserialize IVC proof: {err}"))
        })?;
    nova_params
        .verify(ivc_proof.clone())
        .map_err(|err| ProverError::InvalidInput(format!("invalid IVC proof: {err}")))?;
    let nova = nova_params.nova_from_ivc_proof(ivc_proof)?;
    decider_params
        .generate_decider_proof(nova)
        .map_err(Into::into)
}

fn read_artifact(dir: &Path, prefix: &str, suffix: &str) -> Result<Vec<u8>, ProverError> {
    let path = dir.join(format!("{prefix}_{suffix}"));
    fs::read(&path).map_err(|err| {
        ProverError::Io(std::io::Error::new(
            err.kind(),
            format!("failed to read {}: {}", path.display(), err),
        ))
    })
}
