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
  no hook for external SRS; if "all Groth16 uses ptau" is required, we need a patch/fork.
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
5. If decider Groth16 must use ptau, patch folding-schemes to accept injected
   Groth16 keys (or add a custom decider wrapper).
6. Validate by generating proofs and verifying with existing contracts.

## Open questions
- Select the ptau source with >= 2^24 power (largest requirement comes from decider Groth16).
- ptau source decided: `ppot_0080_24.ptau` (power=24) from
  `https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_24.ptau`.
- File size is ~18GB, so it will be downloaded on demand and not committed.
