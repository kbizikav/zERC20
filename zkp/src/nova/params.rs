use std::io::Cursor;

use ark_bn254::{Bn254, Fr, G1Projective as G1};
use ark_groth16::Groth16;
use ark_grumpkin::Projective as G2;
use ark_serialize::CanonicalDeserialize;
use ark_serialize::{CanonicalSerialize as _, Compress, Validate};
use folding_schemes::folding::nova::PreprocessorParam;
use folding_schemes::folding::traits::CommittedInstanceOps as _;
use folding_schemes::transcript::poseidon::poseidon_canonical_config;
use folding_schemes::{
    Decider, FoldingScheme,
    commitment::{kzg::KZG, pedersen::Pedersen},
    folding::nova::{IVCProof, Nova, decider_eth::Decider as DeciderEth},
    frontend::FCircuit,
};
use rand::rngs::OsRng;
use solidity_verifiers::utils::eth::ToEth;
use solidity_verifiers::{NovaCycleFoldVerifierKey, get_decider_template_for_cyclefold_decider};

#[derive(thiserror::Error, Debug)]
pub enum NovaError {
    #[error("Deserialization Error: {0}")]
    DeserializationError(String),

    #[error("Serialization Error: {0}")]
    SerializationError(String),

    #[error("Initialization Error: {0}")]
    InitializationError(String),

    #[error("Verification Error: {0}")]
    VerificationError(String),

    #[error("Preprocess Error: {0}")]
    PreprocessError(String),

    #[error("Decider Proof Error: {0}")]
    DeciderProofError(String),
}

pub type N<C> = Nova<G1, G2, C, KZG<'static, Bn254>, Pedersen<G2>, false>;
pub type D<C> = DeciderEth<G1, G2, C, KZG<'static, Bn254>, Pedersen<G2>, Groth16<Bn254>, N<C>>;

pub type FParams<C> = <C as FCircuit<Fr>>::Params;
pub type NovaPP<C> = <N<C> as FoldingScheme<G1, G2, C>>::ProverParam;
pub type NovaVP<C> = <N<C> as FoldingScheme<G1, G2, C>>::VerifierParam;
pub type DeciderPP<C> = <D<C> as Decider<G1, G2, C, N<C>>>::ProverParam;
pub type DeciderVP<C> = <D<C> as Decider<G1, G2, C, N<C>>>::VerifierParam;

pub struct NovaParams<C: FCircuit<Fr>>
where
    FParams<C>: Clone,
{
    pub f_params: FParams<C>,
    pub pp: NovaPP<C>,
    pub vp: NovaVP<C>,
}

impl<C: FCircuit<Fr>> NovaParams<C>
where
    FParams<C>: Clone,
{
    pub fn rand<R: rand::RngCore>(f_params: FParams<C>, rng: &mut R) -> Result<Self, NovaError> {
        let circuit = C::new(f_params.clone()).map_err(|e| {
            NovaError::InitializationError(format!("FCircuit Initialization: {}", e))
        })?;
        let poseidon_config = poseidon_canonical_config::<Fr>();
        let preprocess_params =
            PreprocessorParam::<G1, G2, C, KZG<'static, Bn254>, Pedersen<G2>, false>::new(
                poseidon_config,
                circuit.clone(),
            );
        let nova_params = N::preprocess(rng, &preprocess_params)
            .map_err(|e| NovaError::PreprocessError(format!("Nova Preprocess: {}", e)))?;
        Ok(Self {
            f_params,
            pp: nova_params.0,
            vp: nova_params.1,
        })
    }

    pub fn from_bytes(
        f_params: FParams<C>,
        pp_bytes: Vec<u8>,
        vp_bytes: Vec<u8>,
    ) -> Result<Self, NovaError> {
        let nova_pp = {
            let mut cur = Cursor::new(&pp_bytes);
            N::<C>::pp_deserialize_with_mode(
                &mut cur,
                Compress::No,
                Validate::Yes,
                f_params.clone(),
            )
            .map_err(|e| NovaError::DeserializationError(format!("Nova Prover Params: {}", e)))?
        };
        let nova_vp = {
            let mut rd = Cursor::new(&vp_bytes);
            N::<C>::vp_deserialize_with_mode(&mut rd, Compress::No, Validate::Yes, f_params.clone())
                .map_err(|e| {
                    NovaError::DeserializationError(format!("Nova Verifier Params: {}", e))
                })?
        };
        Ok(Self {
            f_params,
            pp: nova_pp,
            vp: nova_vp,
        })
    }

    pub fn to_bytes(&self) -> Result<(Vec<u8>, Vec<u8>), NovaError> {
        let mut pp_bytes = Vec::new();
        self.pp
            .serialize_with_mode(&mut pp_bytes, Compress::No)
            .map_err(|e| NovaError::SerializationError(format!("Nova Prover Params: {}", e)))?;
        let mut vp_bytes = Vec::new();
        self.vp
            .serialize_with_mode(&mut vp_bytes, Compress::No)
            .map_err(|e| NovaError::SerializationError(format!("Nova Verifier Params: {}", e)))?;
        Ok((pp_bytes, vp_bytes))
    }

    pub fn initial_nova(&self, z0: Vec<Fr>) -> Result<N<C>, NovaError> {
        N::<C>::init(
            &(self.pp.clone(), self.vp.clone()),
            C::new(self.f_params.clone()).map_err(|e| {
                NovaError::InitializationError(format!("FCircuit Initialization: {}", e))
            })?,
            z0,
        )
        .map_err(|e| NovaError::InitializationError(format!("Nova Initialization: {}", e)))
    }

    pub fn nova_from_ivc_proof(&self, ivc_proof: IVCProof<G1, G2>) -> Result<N<C>, NovaError> {
        N::<C>::from_ivc_proof(
            ivc_proof,
            self.f_params.clone(),
            (self.pp.clone(), self.vp.clone()),
        )
        .map_err(|e| NovaError::InitializationError(format!("Nova from IVC Proof: {}", e)))
    }

    pub fn verify(&self, ivc_proof: IVCProof<G1, G2>) -> Result<(), NovaError> {
        N::<C>::verify(self.vp.clone(), ivc_proof)
            .map_err(|e| NovaError::VerificationError(format!("Nova Verification: {}", e)))?;
        Ok(())
    }

    pub fn state_len(&self) -> Result<usize, NovaError> {
        let circuit = C::new(self.f_params.clone()).map_err(|e| {
            NovaError::InitializationError(format!("FCircuit Initialization: {}", e))
        })?;
        Ok(circuit.state_len())
    }
}

pub struct DeciderParams<C: FCircuit<Fr>>
where
    FParams<C>: Clone,
{
    pub pp: DeciderPP<C>,
    pub vp: DeciderVP<C>,
}

impl<C: FCircuit<Fr>> DeciderParams<C>
where
    FParams<C>: Clone,
{
    pub fn rand<R: rand::RngCore + rand::CryptoRng>(
        rng: &mut R,
        nova_params: &NovaParams<C>,
    ) -> Result<Self, NovaError> {
        let decider_params = D::<C>::preprocess(
            rng,
            (
                (nova_params.pp.clone(), nova_params.vp.clone()),
                nova_params.state_len()?,
            ),
        )
        .map_err(|e| NovaError::PreprocessError(format!("Decider Preprocess: {}", e)))?;
        Ok(Self {
            pp: decider_params.0,
            vp: decider_params.1,
        })
    }

    pub fn from_bytes(pp_bytes: Vec<u8>, vp_bytes: Vec<u8>) -> Result<Self, NovaError> {
        let decider_pp = {
            let mut rd = Cursor::new(&pp_bytes);
            DeciderPP::<C>::deserialize_uncompressed(&mut rd).map_err(|e| {
                NovaError::DeserializationError(format!("Decider Prover Params: {}", e))
            })?
        };
        let decider_vp = {
            let mut rd = Cursor::new(&vp_bytes);
            DeciderVP::<C>::deserialize_uncompressed(&mut rd).map_err(|e| {
                NovaError::DeserializationError(format!("Decider Verifier Params: {}", e))
            })?
        };
        Ok(Self {
            pp: decider_pp,
            vp: decider_vp,
        })
    }

    pub fn to_bytes(&self) -> Result<(Vec<u8>, Vec<u8>), NovaError> {
        let mut pp_bytes = Vec::new();
        self.pp
            .serialize_with_mode(&mut pp_bytes, Compress::No)
            .map_err(|e| NovaError::SerializationError(format!("Decider Prover Params: {}", e)))?;
        let mut vp_bytes = Vec::new();
        self.vp
            .serialize_with_mode(&mut vp_bytes, Compress::No)
            .map_err(|e| {
                NovaError::SerializationError(format!("Decider Verifier Params: {}", e))
            })?;
        Ok((pp_bytes, vp_bytes))
    }

    pub fn verifier_solidity_code(&self, state_len: usize) -> String {
        let nova_cyclefold_vk = NovaCycleFoldVerifierKey::from((self.vp.clone(), state_len));
        get_decider_template_for_cyclefold_decider(nova_cyclefold_vk)
    }

    pub fn generate_decider_proof(&self, nova: N<C>) -> Result<Vec<u8>, NovaError> {
        let mut rng = OsRng;
        let proof = D::<C>::prove(&mut rng, self.pp.clone(), nova.clone()).map_err(|e| {
            NovaError::DeciderProofError(format!("Decider Proof Generation: {}", e))
        })?;

        // verify the proof
        let verified = D::<C>::verify(
            self.vp.clone(),
            nova.i,
            nova.z_0.clone(),
            nova.z_i.clone(),
            &nova.U_i.get_commitments(),
            &nova.u_i.get_commitments(),
            &proof,
        )
        .map_err(|e| NovaError::DeciderProofError(format!("Decider Proof Verification: {}", e)))?;
        if !verified {
            return Err(NovaError::DeciderProofError(
                "Decider Proof Verification Failed".to_string(),
            )
            .into());
        }

        // generate calldata from the proof
        let proof = [
            nova.i.to_eth(),   // i
            nova.z_0.to_eth(), // z_0
            nova.z_i.to_eth(), // z_i
            nova.U_i.cmW.to_eth(),
            nova.U_i.cmE.to_eth(),
            nova.u_i.cmW.to_eth(),
            proof.cmT().to_eth(),                 // cmT
            proof.r().to_eth(),                   // r
            proof.snark_proof().to_eth(),         // pA, pB, pC
            proof.kzg_challenges().to_eth(),      // challenge_W, challenge_E
            proof.kzg_proofs()[0].eval.to_eth(),  // eval W
            proof.kzg_proofs()[1].eval.to_eth(),  // eval E
            proof.kzg_proofs()[0].proof.to_eth(), // W kzg_proof
            proof.kzg_proofs()[1].proof.to_eth(), // E kzg_proof
        ]
        .concat();

        Ok(proof)
    }
}
