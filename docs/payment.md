## Summary

In **zERC20**, tokens are sent to a **burn address** that appears unrelated to the actual recipient.  
The recipient can later **withdraw** the funds using secret information, ensuring **transaction privacy**.

There are two types of withdrawals:

- **Single Withdraw:**  
  When there is only one transfer with the same `(recipient_chain_id, recipient_address, tweak)`.  
  → Completes within a few seconds in WASM.

- **Batch Withdraw:**  
  When there are multiple transfers with the same `(recipient_chain_id, recipient_address, tweak)`.  
  → Requires a WASM proof (~2 seconds per transfer) plus a heavy **decider proof** (~30 seconds server-side).

---

## Terminology

| Term             | Description                                                                                                                          |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| **Burn address** | An address treated as “burned” when tokens are sent to it. <br>`burn = H( H(recipient_chain_id, recipient_address, tweak), secret )` |
| **seed**         | A secret value derived from the user’s signature. Used to derive `tweak` and `secret`.                                               |
| **tweak**        | A recipient-linked derived parameter used for collision avoidance and batch keying.                                                  |
| **secret**       | A private parameter determining withdrawability. Only the holder can withdraw.                                                       |

---

## Information Exchange Methods

There are two ways for the sender and recipient to exchange the `burn address` and related secret data.

---

### A. Invoice (Recipient-initiated)

**Purpose:**  
The recipient issues an invoice, and the sender transfers tokens to the specified burn address.

#### Steps

1. **Recipient:**  
   Chooses an `invoice_id` and submits it to the Internet Computer storage canister (via `submit_invoice`).  
   Embeds a single/multiple flag (`is_batch`) in the `invoice_id` bits.

2. **Recipient:**

```

(tweak, salt) = DeriveTweakAndSalt(seed_R, invoice_id)
secret = DeriveSecret(view_pub_R, salt)
burn = BurnAddress(rcid, raddr, tweak, secret)

```

3. **Recipient → Sender:**  
   Sends the generated `burn` address to the sender (this is the **invoice**).

4. **Sender:**  
   Sends tokens to the `burn` address.

5. **Recipient:**  
   Checks payment completion via `token.balanceOf(burn)`.

6. **Withdrawal:**

- If `is_batch == false` → **Single Withdraw**
- If `is_batch == true` → Scan `sub_id ∈ [0..n]` and perform **Batch Withdraw**

---

### B. Payment Advice (Sender-initiated)

**Purpose:**  
The sender generates the recipient’s burn address on their side, sends tokens,  
and then off-chain notifies the recipient of the secret parameters used.

#### Steps

1. **Sender:**  
   Obtains recipient’s `(rcid, raddr)`.

2. **Sender:**

```

(tweak, secret) = DeriveTweakAndSalt(seed_S, advice_id)
burn = BurnAddress(rcid, raddr, tweak, secret)

```

Generates the burn address and sends tokens.

3. **Sender → Recipient:**  
   Sends `(secret, tweak)` (and optionally `(rcid, raddr)`) off-chain.  
   → This is called a **payment advice**.

4. **Recipient:**  
   Uses the received `tweak` and `secret` to perform a **Single Withdraw**.
