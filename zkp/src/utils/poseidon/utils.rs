use ark_crypto_primitives::sponge::{
    Absorb, CryptographicSponge, FieldBasedCryptographicSponge,
    poseidon::{PoseidonConfig, PoseidonSponge},
};
use ark_ff::PrimeField;
use light_poseidon::PoseidonParameters;

/// Translate light-poseidon parameters into an arkworks Poseidon configuration.
pub fn light_poseidon_to_ark_config<F: PrimeField>(
    params: &PoseidonParameters<F>,
) -> PoseidonConfig<F> {
    let width = params.width;
    let rounds = params.full_rounds + params.partial_rounds;
    assert_eq!(params.ark.len(), rounds * width, "round constants length");

    let ark = params
        .ark
        .chunks(width)
        .map(|chunk| chunk.to_vec())
        .collect();

    PoseidonConfig::new(
        params.full_rounds,
        params.partial_rounds,
        params.alpha,
        params.mds.clone(),
        ark,
        width - 1,
        1,
    )
}

pub fn circom_poseidon_config<F: PrimeField + From<ark_ff::BigInt<4>>>() -> PoseidonConfig<F> {
    let params = light_poseidon::parameters::bn254_x5::get_poseidon_parameters::<F>(3)
        .expect("poseidon parameters");
    light_poseidon_to_ark_config(&params)
}

/// Compute a Poseidon hash using arkworks' Poseidon sponge configured with `config`.
pub fn circom_poseidon_hash<F: PrimeField + Absorb>(config: &PoseidonConfig<F>, inputs: &[F]) -> F {
    assert_eq!(
        inputs.len(),
        config.rate,
        "inputs must fill the sponge rate",
    );

    let mut sponge = PoseidonSponge::<F>::new(config);
    for input in inputs {
        sponge.absorb(input);
    }

    let _ = sponge.squeeze_native_field_elements(1);
    sponge.state[0]
}

pub fn poseidon2<F: PrimeField + Absorb + From<ark_ff::BigInt<4>>>(left: F, right: F) -> F {
    let config = circom_poseidon_config();
    circom_poseidon_hash(&config, &[left, right])
}
