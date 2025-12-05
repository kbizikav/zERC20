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

pub fn circom_poseidon_config_with_rate<F: PrimeField + From<ark_ff::BigInt<4>>>(
    rate: usize,
) -> PoseidonConfig<F> {
    assert!(rate >= 1, "poseidon rate must be at least 1");
    let width = rate
        .checked_add(1)
        .expect("poseidon width computation should not overflow");
    let width = u8::try_from(width).expect("poseidon width must fit into u8");
    let params = light_poseidon::parameters::bn254_x5::get_poseidon_parameters::<F>(width)
        .expect("poseidon parameters");
    light_poseidon_to_ark_config(&params)
}

pub fn circom_poseidon2_config<F: PrimeField + From<ark_ff::BigInt<4>>>() -> PoseidonConfig<F> {
    circom_poseidon_config_with_rate(2)
}

pub fn circom_poseidon3_config<F: PrimeField + From<ark_ff::BigInt<4>>>() -> PoseidonConfig<F> {
    circom_poseidon_config_with_rate(3)
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
    let config = circom_poseidon2_config();
    circom_poseidon_hash(&config, &[left, right])
}

pub fn poseidon3<F: PrimeField + Absorb + From<ark_ff::BigInt<4>>>(
    left: F,
    middle: F,
    right: F,
) -> F {
    let config = circom_poseidon3_config();
    circom_poseidon_hash(&config, &[left, middle, right])
}
