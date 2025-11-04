use std::io::Cursor;

use ark_bn254::{Bn254, Fr};
use ark_groth16::{Groth16, ProvingKey, VerifyingKey};
use ark_relations::gr1cs::ConstraintSynthesizer;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize, Compress};
use ark_snark::{CircuitSpecificSetupSNARK, SNARK};
use rand::{CryptoRng, RngCore};
use solidity_verifiers::utils::eth::ToEth;
use solidity_verifiers::{Groth16VerifierKey, ProtocolVerifierKey};

#[derive(thiserror::Error, Debug)]
pub enum Groth16Error {
    #[error("Setup Error: {0}")]
    SetupError(String),
    #[error("Serialization Error: {0}")]
    SerializationError(String),
    #[error("Deserialization Error: {0}")]
    DeserializationError(String),
    #[error("Solidity Export Error: {0}")]
    SolidityExportError(String),
    #[error("Proof Generation Error: {0}")]
    ProofGenerationError(String),
    #[error("Proof Verification Internal Error: {0}")]
    ProofVerificationInternalError(String),
    #[error("Proof Verification Error: {0}")]
    ProofVerificationError(String),
}

#[derive(Clone)]
pub struct Groth16Params {
    pub vk: VerifyingKey<Bn254>,
    pub pk: ProvingKey<Bn254>,
}

impl Groth16Params {
    pub fn rand<C, R>(rng: &mut R, circuit: C) -> Result<Self, Groth16Error>
    where
        C: ConstraintSynthesizer<Fr>,
        R: RngCore + CryptoRng,
    {
        let (pk, vk) = Groth16::<Bn254>::setup(circuit, rng)
            .map_err(|e| Groth16Error::SetupError(e.to_string()))?;
        Ok(Self { vk, pk })
    }

    pub fn from_bytes(pk_bytes: Vec<u8>, vk_bytes: Vec<u8>) -> Result<Self, Groth16Error> {
        let pk = {
            let mut rd = Cursor::new(&pk_bytes);
            ProvingKey::<Bn254>::deserialize_uncompressed(&mut rd).map_err(|e| {
                Groth16Error::DeserializationError(format!("Groth16 Proving Key: {}", e))
            })?
        };

        let vk = {
            let mut rd = Cursor::new(&vk_bytes);
            VerifyingKey::<Bn254>::deserialize_uncompressed(&mut rd).map_err(|e| {
                Groth16Error::DeserializationError(format!("Groth16 Verifying Key: {}", e))
            })?
        };

        Ok(Self { vk, pk })
    }

    pub fn to_bytes(&self) -> Result<(Vec<u8>, Vec<u8>), Groth16Error> {
        let mut pk_bytes = Vec::new();
        self.pk
            .serialize_with_mode(&mut pk_bytes, Compress::No)
            .map_err(|e| Groth16Error::SerializationError(format!("Groth16 Proving Key: {}", e)))?;

        let mut vk_bytes = Vec::new();
        self.vk
            .serialize_with_mode(&mut vk_bytes, Compress::No)
            .map_err(|e| {
                Groth16Error::SerializationError(format!("Groth16 Verifying Key: {}", e))
            })?;

        Ok((pk_bytes, vk_bytes))
    }

    pub fn verifier_solidity_code(&self) -> Result<String, Groth16Error> {
        let verifier_key = Groth16VerifierKey::from(self.vk.clone());
        let bytes = verifier_key.render_as_template(None);
        String::from_utf8(bytes)
            .map_err(|e| Groth16Error::SolidityExportError(format!("UTF-8 Error: {}", e)))
    }

    pub fn generate_proof<C, R>(
        &self,
        rng: &mut R,
        circuit: C,
        public_inputs: &[Fr],
    ) -> Result<Vec<u8>, Groth16Error>
    where
        C: ConstraintSynthesizer<Fr>,
        R: RngCore + CryptoRng,
    {
        let proof = Groth16::<Bn254>::prove(&self.pk, circuit, rng)
            .map_err(|e| Groth16Error::ProofGenerationError(e.to_string()))?;

        let verified = Groth16::<Bn254>::verify(&self.vk, public_inputs, &proof).map_err(|e| {
            Groth16Error::ProofVerificationInternalError(format!(
                "Groth16 proof verification: {}",
                e
            ))
        })?;

        if !verified {
            return Err(Groth16Error::ProofVerificationError(
                "Groth16 proof verification failed".to_string(),
            ));
        }

        Ok([proof.to_eth(), public_inputs.to_eth()].concat())
    }
}
