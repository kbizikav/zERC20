# Cryptography feasibility (ptau + phase2)

## Feasibility summary
- Using a perpetualpowersoftau .ptau for Groth16 phase1 is supported by arkworks-phase2
  (`Accumulator::from_ptau_file`).
- KZG SRS can be derived from the same ptau and injected into folding-schemes via
  `PreprocessorParam` (cs_pp/cs_vp), so Nova can avoid RNG-based setup.
- Groth16 phase2 contributions are supported by arkworks-phase2 `Transcript` and emit
  standard ark-groth16 proving/verifying keys.
- Nova decider Groth16 must be included in the ptau-based setup (confirmed).

## Constraints and risks
- folding-schemes `DeciderEth` uses `Groth16::circuit_specific_setup` internally with
  no hook for external SRS; we will mirror that preprocess logic in trusted-setup-cli
  to inject Groth16 keys without modifying Sonobe.
- ptau size must cover the max constraint count for:
  - SingleWithdraw circuits (local/global)
  - Nova and CycleFold R1CS (for KZG SRS length)
  - DeciderEth circuit (if Groth16 ceremony is required there)
- KZG verifier key needs g1 CRS batch points (e.g., first 3 powers) for solidity templates.
- Deterministic RNG is acceptable for Pedersen params (confirmed).

## Circuit size measurements (from log_circuit_sizes)
- Groth16 single-withdraw:
  - local: constraints=14583, total_with_instance=14587 -> ptau >= 2^14
  - global: constraints=16035, total_with_instance=16039 -> ptau >= 2^14
- Nova (augmented) + CycleFold:
  - root: constraints=162177 -> KZG len=162177 (pow2=262144, 2^18)
  - withdraw_local: constraints=74416 -> KZG len=74416 (pow2=131072, 2^17)
  - withdraw_global: constraints=75850 -> KZG len=75850 (pow2=131072, 2^17)
  - cyclefold: constraints=1349 -> Pedersen len=1349 (pow2=2048, 2^11)
- Nova decider Groth16:
  - root: constraints=9,568,050, total_with_instance=9,568,093 -> ptau >= 2^24
  - withdraw_local: constraints=8,887,659, total_with_instance=8,887,704 -> ptau >= 2^24
  - withdraw_global: constraints=8,894,895, total_with_instance=8,894,940 -> ptau >= 2^24

## Proposed approach (crypto part)
1. Measure constraint counts for all relevant circuits to select an appropriate ptau power.
2. Add a helper to load ptau with arkworks-phase2 and derive:
   - Groth16 phase1 accumulator
   - KZG ProverKey/VerifierKey from tau powers (g1/g2, beta_h)
3. Extend zkp Nova setup to accept external KZG params (cs_pp/cs_vp) from ptau.
4. Replace Groth16 `setup` in artifact generation with arkworks-phase2 `Transcript`
   (no contribution) and serialize pk/vk.
5. For decider Groth16, mirror `DeciderEth::preprocess` in `trusted-setup/cli` and
   inject Groth16 keys (no Sonobe API changes).
6. Validate by generating proofs and verifying with existing contracts.

## Resolved decisions
- ptau power: 2^24 (largest requirement comes from decider Groth16).
- ptau source: `ppot_0080_24.ptau` from
  `https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_24.ptau`.
- File size is ~18GB, so it will be downloaded on demand and not committed.

## Design overview (trusted-setup)
- Layout under `<repo-root>/trusted-setup/`:
  - `trusted-setup/cli`: participant tool (ptau fetch + contribute + finalize).
  - `trusted-setup/coordinator`: actix-web server that coordinates contributions,
    verifies transitions, manages time locks, and issues presigned URLs.
- No Sonobe API changes: the CLI constructs Nova params with injected KZG SRS and
  mirrors `DeciderEth::preprocess` to build decider params externally.
- Deterministic RNG for Pedersen params (seeded `StdRng`) is acceptable.

## Arkworks-phase2 usage (detailed)
- Load ptau: `Accumulator::<Bn254>::from_ptau_file(ptau_path)`.
- Groth16 phase2 (circuit-specific) key generation:
  1) Build the circuit (single-withdraw or decider).
  2) `Transcript::new_from_accumulator(&accum, circuit)` to derive initial key.
  3) Optional: `transcript.verify_from_accumulator(&accum, circuit)` before use.
  4) Output `ProvingKey`/`VerifyingKey` from `transcript.key` for artifacts.
- Groth16 phase2 contribution flow:
  - Load transcript bytes, verify against accumulator/circuit.
  - `transcript.contribute_seed(...)` or `contribute_rng(...)`.
  - `transcript.verify()` and emit updated transcript bytes plus public contribution data.

## KZG SRS derivation from ptau (detailed)
- Use ptau tau powers as KZG powers (non-hiding):
  - `powers_of_g[i] = tau_powers_g1[i]` for i = 0..=len.
  - `h = tau_powers_g2[0]`, `beta_h = tau_powers_g2[1]` (tau is the KZG beta).
  - `gamma_g = alpha_tau_powers_g1[0]` and `powers_of_gamma_g[i] = alpha_tau_powers_g1[i]`
    to keep KZG10 `VerifierKey` consistent (gamma_g is unused in non-hiding checks).
- Build `KZG` ProverParams/VerifierParams for folding-schemes:
  - ProverParams = `ProverKey { powers_of_g }`.
  - VerifierParams = `VerifierKey { g, gamma_g, h, beta_h, prepared_h, prepared_beta_h }`.
- Slice length based on `max(r1cs.n_constraints(), r1cs.n_witnesses())` for each circuit.

## Nova params injection (no Sonobe changes)
- Construct `PreprocessorParam` with `cs_pp/cs_vp` set to KZG params derived above.
- Run `Nova::preprocess` to get `nova_pp/nova_vp`.
- Pedersen params for CycleFold: deterministic `StdRng::seed_from_u64(...)`.

## Decider params construction (mirror DeciderEth::preprocess)
- Build `DeciderEthCircuit::dummy` with:
  - `nova_vp.r1cs`, `nova_vp.cf_r1cs`, `nova_pp.cf_cs_pp`, `nova_pp.poseidon_config`,
    `state_len`, `num_commitments=2`.
- Generate Groth16 keys for the decider circuit via arkworks-phase2 `Transcript`.
- Assemble decider params:
  - `decider_pp = (g16_pk, nova_pp.cs_pp)`
  - `decider_vp = { pp_hash: nova_vp.pp_hash(), snark_vp: g16_vk, cs_vp: nova_vp.cs_vp }`
- Serialize to the existing artifact layout so decider-prover can load without changes.

## CLI design (rough)
### Contribution flow (coordinator-led, p0tion-like)
- Storage: S3 for `transcript` and `contribution` artifacts; S3 reads are public
  (or otherwise accessible without coordinator). Coordinator issues presigned PUTs
  for uploads and updates head metadata.
- Coordinator steps:
  1) Participant requests a slot; coordinator creates a time lock (lease) in SQLite.
  2) Coordinator returns presigned GET for current transcript, presigned PUTs for
     updated transcript + contribution data, and lease expiry time.
  3) Participant uploads and calls `submit`.
  4) Coordinator downloads the new transcript, verifies it using
     `Transcript::verify_from_accumulator` (ptau + circuit), then optionally checks
     `Transcript::verify_key_transform` against the previous transcript, updates head
     metadata, and releases the lock.
- Participant steps:
  1) `trusted-setup-cli contribute` asks coordinator for a slot.
  2) CLI downloads ptau (if missing), verifies from accumulator + input transcript,
     applies the contribution, uploads transcript + public data, then submits.
- Finalization (anyone, no coordinator required for download):
  1) `trusted-setup-cli finalize` reads `latest.json` (or a user-specified transcript)
     directly from S3.
  2) CLI verifies from accumulator + transcript, then emits all `*.bin` and `*.sol` outputs.

### Commands (subject to change)
- `ptau download` (cache to a local path, no commit)
- `contribute` (verify from accumulator + transcript, apply contribution, emit updated transcript)
- `finalize` (verify from accumulator + transcript, emit all artifacts)

### Config (rough)
- Configuration via environment variables (preferred over CLI args):
  - ptau path (default to cached file)
  - coordinator URL and ceremony ID
  - output dir
  - circuit selection (root/withdraw_local/withdraw_global)
  - deterministic seed or entropy source
  - S3 public read base URL (for latest.json/transcripts)

## Coordinator server design (rough)
- Server: actix-web with SQLite state.
- State model (SQLite):
  - ceremony (id, status, current_head_key, lease_ttl)
  - lease (participant_id, started_at, expires_at, status)
  - contribution (step, participant_id, input_key, output_key, proof_key, status)
- Time lock: only one active lease per ceremony; expiry unlocks the slot.
- Verification:
  - Load previous and new transcript from S3.
  - Run `Transcript::verify_from_accumulator(&accum, circuit)` on the new transcript.
  - On success, update head metadata and mark contribution complete.
- Server config (env-driven):
  - S3 bucket/prefix, presign TTL, AWS credentials or role, SQLite path, listen address.
  - Public read policy for transcript objects and `latest.json`.

## Outputs / compatibility
- Keep existing artifact names so downstream code works:
  - `*_groth16_pk.bin`, `*_groth16_vk.bin`
  - `*_nova_pp.bin`, `*_nova_vp.bin`
  - `*_decider_pp.bin`, `*_decider_vp.bin`
- Also emit Solidity verifier contracts for Groth16 and Nova decider:
  - `{Prefix}Groth16Verifier.sol`
  - `{Prefix}NovaDecider.sol`
