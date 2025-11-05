## Summary

In **zERC20**, stealth transfers are implemented by deriving a sealed **stealth payload** that hides
the real recipient. Funds move through burn addresses, while the withdrawable secret material remains
inside the encrypted payload held by the recipient. Stealth clients typically
communicate with three backends:

1. **Storage canister** on the Internet Computer (stores invoices and encrypted announcements).
2. **Indexer** that exposes ERC‑20 transfers involving burn addresses.
3. **Decider prover + verifier contracts** that validate zero‑knowledge proofs.

Two proving behaviors exist:

- **Single withdraw** — only one transfer shares a tuple `(recipient_chain_id, recipient_address,
  tweak)`. A local WASM prover is sufficient and usually finishes within a few seconds.
- **Batch withdraw** — multiple transfers share the tuple. Each transfer requires a WASM proof plus a
  heavy decider prover session (~30 s server-side) before the final transaction can be submitted.

---

## Terminology

| Term             | Description                                                                                                                                                      |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Burn address** | An address treated as “burned” when tokens are sent to it.<br>`burn = H(H(recipient_chain_id, recipient_address, tweak), secret)`                                 |
| **seed**         | Derived via `compute_seed_from_signature(PRIVATE_KEY)`. Seeds deterministically drive secret/tweak generation for invoices and payment advice IDs.                |
| **tweak**        | Recipient-linked derived parameter that prevents collisions and groups batch members.                                                                            |
| **secret**       | Private parameter stored inside the stealth payload. Ownership of the secret enables withdrawal, and it never leaves the encrypted payload during normal use.     |

---

## Information Exchange Modes

Two complementary workflows cover how senders and recipients coordinate stealth payload
information.

---

### A. Invoice (Recipient-initiated)

Recipients register invoices with the storage canister, distribute the derived burn addresses to
payers, and later redeem the resulting transfers.

1. **Issuing** — the recipient chooses single or batch mode and signs a `submit_invoice` request that
   persists a unique `invoice_id`. Seeds derived from the signing key deterministically produce the
   `(tweak, secret)` pairs used for burn address derivation.
2. **Distribution** — the recipient hands the resulting burn address (single mode) or a subset of the
   10 batch burn addresses (sub IDs `0..9`) to potential payers. Any ERC‑20 transfer directed to
   those burn addresses counts toward the invoice.
3. **Monitoring** — the recipient (or an automated agent) rebuilds the candidate burn addresses,
   queries the indexer for matching transfers, and classifies each transfer as eligible or
   ineligible using the on-chain aggregation tree state.
4. **Redemption** — once eligible transfers exist, the recipient proves ownership by supplying the
   secret material inside the stealth payloads, generates proofs (single or batch), and
   submits calls to the verifier contracts. Batch withdrawals also upload witness data to the
   decider prover before finalization.

---

### B. Payment Advice (Sender-initiated)

Senders can originate stealth transfers without waiting for an invoice by deriving a burn address,
publishing the encrypted payload, and transferring funds immediately.

1. **Derivation and funding**
   - The sender samples a `payment_advice_id`, derives `(tweak, secret)` from their seed, and builds
     the stealth payload for the intended `(recipient_chain_id, recipient_address)`.
   - The resulting payload encodes the burn address along with the proof secret material. The sender
     transfers ERC‑20 funds directly to that burn address.
2. **Announcement**
   - Before (or immediately after) funding, the sender fetches the recipient’s registered view public
     key from the storage canister, encrypts the stealth payload, and submits the ciphertext as an
     announcement. Each announcement receives an ID plus timestamps, enabling recipients to collect
     pending transfers later.
   - As an alternative, the sender may deliver the serialized stealth payload to the recipient via
     an out-of-band channel instead of relying on announcements.
3. **Recipient collection**
   - Recipients authenticate with the key-manager canister, download batched announcements, decrypt
     those targeted to them, and persist the recovered `full_burn_address_hex`, `burn_address`, and
     metadata locally.
4. **Redemption**
   - Using either the stored announcement ID or a raw payload blob, the recipient loads the
     stealth payload, verifies that eligible transfers exist via the indexer, and runs the same
     proof pipeline described in the invoice workflow. If no eligible events remain, no proofs are
     attempted.

---

## Operational Notes

- Recipients must register a view public key with the storage canister so that payment advice flows
  can publish encrypted announcements.
- Stealth payloads embed the withdrawal secret. Treat any serialized payload or decrypted
  announcement as confidential and avoid sharing it beyond the intended parties.
- Single withdrawals complete entirely within local WASM proving, while batch withdrawals require a
  cooperative decider prover service and the corresponding Nova artifacts.
