use solana_bn254::prelude::{
    alt_bn128_addition, alt_bn128_multiplication, alt_bn128_pairing,
};

#[derive(PartialEq, Eq, Debug)]
pub struct Groth16VerifyingKey<'a> {
    pub nr_pubinputs: usize,
    pub vk_alpha_g1: [u8; 64],
    pub vk_beta_g2: [u8; 128],
    pub vk_gamma_g2: [u8; 128],
    pub vk_delta_g2: [u8; 128],
    pub vk_ic: &'a [[u8; 64]],
}

#[derive(PartialEq, Eq, Debug)]
pub struct Groth16Verifier<'a, const NR_INPUTS: usize> {
    proof_a: &'a [u8; 64],
    proof_b: &'a [u8; 128],
    proof_c: &'a [u8; 64],
    public_inputs: &'a [[u8; 32]; NR_INPUTS],
    prepared_public_inputs: [u8; 64],
    verifying_key: &'a Groth16VerifyingKey<'a>,
}

#[derive(Debug)]
pub enum Groth16Error {
    InvalidG1Length,
    InvalidG2Length,
    InvalidPublicInputsLength,
    PreparingInputsG1MulFailed,
    PreparingInputsG1AdditionFailed,
    ProofVerificationFailed,
}

impl core::fmt::Display for Groth16Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let msg = match self {
            Groth16Error::InvalidG1Length => "invalid G1 length",
            Groth16Error::InvalidG2Length => "invalid G2 length",
            Groth16Error::InvalidPublicInputsLength => "invalid public inputs length",
            Groth16Error::PreparingInputsG1MulFailed => "G1 mul failed while preparing inputs",
            Groth16Error::PreparingInputsG1AdditionFailed => "G1 add failed while preparing inputs",
            Groth16Error::ProofVerificationFailed => "pairing check failed",
        };
        f.write_str(msg)
    }
}

impl<const NR_INPUTS: usize> Groth16Verifier<'_, NR_INPUTS> {
    pub fn new<'a>(
        proof_a: &'a [u8; 64],
        proof_b: &'a [u8; 128],
        proof_c: &'a [u8; 64],
        public_inputs: &'a [[u8; 32]; NR_INPUTS],
        verifying_key: &'a Groth16VerifyingKey<'a>,
    ) -> Result<Groth16Verifier<'a, NR_INPUTS>, Groth16Error> {
        if proof_a.len() != 64 {
            return Err(Groth16Error::InvalidG1Length);
        }
        if proof_b.len() != 128 {
            return Err(Groth16Error::InvalidG2Length);
        }
        if proof_c.len() != 64 {
            return Err(Groth16Error::InvalidG1Length);
        }
        if public_inputs.len() + 1 != verifying_key.vk_ic.len() {
            return Err(Groth16Error::InvalidPublicInputsLength);
        }

        Ok(Groth16Verifier {
            proof_a,
            proof_b,
            proof_c,
            public_inputs,
            prepared_public_inputs: [0u8; 64],
            verifying_key,
        })
    }

    fn prepare_inputs(&mut self) -> Result<(), Groth16Error> {
        let mut prepared_public_inputs = self.verifying_key.vk_ic[0];

        for (i, input) in self.public_inputs.iter().enumerate() {
            let mul_res = alt_bn128_multiplication(
                &[&self.verifying_key.vk_ic[i + 1][..], &input[..]].concat(),
            )
            .map_err(|_| Groth16Error::PreparingInputsG1MulFailed)?;
            prepared_public_inputs =
                alt_bn128_addition(&[&mul_res[..], &prepared_public_inputs[..]].concat())
                    .map_err(|_| Groth16Error::PreparingInputsG1AdditionFailed)?[..]
                    .try_into()
                    .map_err(|_| Groth16Error::PreparingInputsG1AdditionFailed)?;
        }

        self.prepared_public_inputs = prepared_public_inputs;
        Ok(())
    }

    pub fn verify_unchecked(&mut self) -> Result<(), Groth16Error> {
        self.prepare_inputs()?;

        let pairing_input = [
            self.proof_a.as_slice(),
            self.proof_b.as_slice(),
            self.prepared_public_inputs.as_slice(),
            self.verifying_key.vk_gamma_g2.as_slice(),
            self.proof_c.as_slice(),
            self.verifying_key.vk_delta_g2.as_slice(),
            self.verifying_key.vk_alpha_g1.as_slice(),
            self.verifying_key.vk_beta_g2.as_slice(),
        ]
        .concat();

        let pairing_res =
            alt_bn128_pairing(pairing_input.as_slice()).map_err(|_| Groth16Error::ProofVerificationFailed)?;
        if pairing_res[31] != 1 {
            return Err(Groth16Error::ProofVerificationFailed);
        }
        Ok(())
    }
}
