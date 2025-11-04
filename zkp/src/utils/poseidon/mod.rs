pub mod circom;
pub mod gadgets;
pub mod utils;

pub use circom::{CircomCRH, CircomTwoToOneCRH};
pub use utils::{circom_poseidon_hash, light_poseidon_to_ark_config};

#[cfg(test)]
mod tests {
    use crate::utils::poseidon::utils::circom_poseidon_config;

    use super::{CircomCRH, CircomTwoToOneCRH, circom_poseidon_hash};
    use ark_bn254::Fr;
    use ark_crypto_primitives::crh::{
        CRHScheme, TwoToOneCRHScheme, poseidon::CRH as ArkPoseidonCRH,
    };
    use ark_ff::AdditiveGroup;
    use light_poseidon::{Poseidon, PoseidonHasher as _};

    #[test]
    fn poseidon_config_alignment() {
        let mut light = Poseidon::<Fr>::new_circom(2).unwrap();

        let config = circom_poseidon_config();

        let zero_inputs = vec![Fr::ZERO; 2];
        let expected = light.hash(&zero_inputs).expect("hash");
        let actual = circom_poseidon_hash(&config, &zero_inputs);
        assert_eq!(expected, actual);

        for offset in 1..=4u64 {
            let inputs: Vec<Fr> = (0..2).map(|i| Fr::from(offset + i as u64)).collect();
            let expected = light.hash(&inputs).expect("hash");
            let actual = circom_poseidon_hash(&config, &inputs);
            assert_eq!(expected, actual);
        }
    }

    #[test]
    fn circom_crh_interfaces_match() {
        let config = circom_poseidon_config();

        let inputs = [Fr::from(1u64), Fr::from(2u64)];

        let mut light = Poseidon::<Fr>::new_circom(2).unwrap();
        let expected = light.hash(&inputs).unwrap();

        let actual = CircomCRH::<Fr>::evaluate(&config, &inputs[..]).unwrap();
        assert_eq!(expected, actual);
    }

    #[test]
    fn circom_two_to_one_matches_light_poseidon() {
        let config = circom_poseidon_config();

        let left = Fr::from(11u64);
        let right = Fr::from(22u64);

        let mut light = Poseidon::<Fr>::new_circom(2).unwrap();
        let expected = light.hash(&[left, right]).unwrap();

        let actual = CircomTwoToOneCRH::<Fr>::evaluate(&config, left, right).unwrap();
        assert_eq!(expected, actual);
    }

    #[test]
    fn circom_crh_differs_from_ark_poseidon() {
        let circom_config = circom_poseidon_config();

        let inputs = [Fr::from(1u64), Fr::from(2u64)];
        let inputs = inputs.as_ref();

        let circom = CircomCRH::<Fr>::evaluate(&circom_config, inputs).unwrap();
        let standard = ArkPoseidonCRH::<Fr>::evaluate(&circom_config, inputs).unwrap();

        assert_ne!(circom, standard);
    }
}
