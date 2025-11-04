use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ic_bls12_381::{
    hash_to_curve::{ExpandMsgXmd, HashToCurve},
    G1Affine, G1Projective, G2Affine, G2Projective, Scalar,
};
use ic_vetkeys::{DerivedPublicKey, IbeCiphertext, IbeIdentity, IbeSeed, VetKey};
use rand::{rngs::StdRng, RngCore, SeedableRng};

const BENCH_IDENTITY: &[u8] = b"zstorage-ibe-decrypt-bench";
const SIGNATURE_DST: &[u8] = b"BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_AUG_";

fn forge_sample(len: usize) -> (Arc<IbeCiphertext>, VetKey, Vec<u8>) {
    let mut rng = StdRng::seed_from_u64(0x51ced215);

    // Deterministic secret ensures reproducible benchmark inputs.
    let sk = Scalar::from(0xdead_beefu64);

    let dpk_projective = G2Projective::generator() * sk;
    let dpk_affine = G2Affine::from(dpk_projective);
    let dpk_bytes = dpk_affine.to_compressed();
    let derived_public_key =
        DerivedPublicKey::deserialize(&dpk_bytes).expect("derived public key is valid");

    let identity = IbeIdentity::from_bytes(BENCH_IDENTITY);

    let mut seed_material = [0u8; 32];
    rng.fill_bytes(&mut seed_material);
    let seed = IbeSeed::from_bytes(&seed_material).expect("seed has sufficient entropy");

    let mut session_key = vec![0u8; len];
    rng.fill_bytes(&mut session_key);

    let ciphertext = IbeCiphertext::encrypt(&derived_public_key, &identity, &session_key, &seed);

    let vetkey = forge_vetkey(&dpk_affine, identity.value(), sk);

    (Arc::new(ciphertext), vetkey, session_key)
}

fn forge_vetkey(dpk: &G2Affine, identity: &[u8], sk: Scalar) -> VetKey {
    let dpk_bytes = dpk.to_compressed();
    let mut input = Vec::with_capacity(dpk_bytes.len() + identity.len());
    input.extend_from_slice(&dpk_bytes);
    input.extend_from_slice(identity);

    let msg = <G1Projective as HashToCurve<ExpandMsgXmd<sha2::Sha256>>>::hash_to_curve(
        input,
        SIGNATURE_DST,
    );
    let signature = G1Affine::from(msg * sk);
    let bytes = signature.to_compressed();
    VetKey::deserialize(&bytes).expect("constructed vetkey is valid")
}

fn benchmark_decrypt(c: &mut Criterion) {
    let mut group = c.benchmark_group("ibe_decrypt_session_key");
    for &len in &[32usize, 64, 256] {
        let (ciphertext, vetkey, expected) = forge_sample(len);

        // Sanity check before measuring.
        let control = ciphertext
            .decrypt(&vetkey)
            .expect("IBE decrypt succeeds for forged inputs");
        assert_eq!(control, expected);

        let bench_cipher = Arc::clone(&ciphertext);
        let bench_vetkey = vetkey.clone();

        group.bench_function(format!("len_{}", len), move |b| {
            b.iter(|| {
                let session_key = bench_cipher
                    .decrypt(&bench_vetkey)
                    .expect("IBE decrypt succeeds");
                black_box(session_key);
            });
        });
    }
    group.finish();
}

criterion_group!(ibe_decrypt, benchmark_decrypt);
criterion_main!(ibe_decrypt);
