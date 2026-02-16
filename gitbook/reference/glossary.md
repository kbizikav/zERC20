# Glossary

Key terms used throughout the zERC20 documentation.

## Core Concepts

### Burn Address
A one-time Ethereum address with no corresponding private key. Tokens sent to a burn address cannot be moved by anyone—they are effectively "burned." The burn address is cryptographically derived from the recipient's withdrawal address and a secret value.

### Private Transfer
A transfer where the link between the sender and recipient is hidden from on-chain observers. In zERC20, this is achieved by sending tokens to a burn address and withdrawing via zero-knowledge proof.

### Zero-Knowledge Proof (ZKP)
A cryptographic method that allows one party to prove they know a value without revealing the value itself. In zERC20, ZKPs prove that a recipient is entitled to withdraw funds from a burn address without revealing which burn address they're withdrawing from.

### Teleport
The on-chain operation that mints tokens to a recipient after a zero-knowledge proof is verified. Corresponds to the `Verifier.teleport()` (batch) and `Verifier.singleTeleport()` (single) contract functions. In effect, teleport is the final step of a private transfer: the proof shows the recipient is entitled to funds burned on any chain, and teleport mints the equivalent amount on the destination chain.

### Withdrawal
The process of claiming tokens from a burn address using a zero-knowledge proof. The tokens are minted to the recipient's chosen address.

## Token Terms

### zERC20
The umbrella term for privacy-enabled ERC-20 wrapper tokens (zUSDC, zETH, zBNB, etc.).

### Wrapper Token
A token that represents another token at a 1:1 ratio. zUSDC is a wrapper for USDC—you deposit USDC and receive an equivalent amount of zUSDC.

### Underlying Token
The original token that backs a zERC20 wrapper. USDC is the underlying token for zUSDC.

## Fees and Liquidity

### Unwrap Fee
A fee charged when converting zERC20 back to the underlying token on chains with low liquidity. Fees are higher when liquidity is further below the target level. No fee is charged when liquidity is at or above target.

### Wrap Reward
A bonus received when wrapping underlying tokens into zERC20 on chains with low liquidity. Rewards incentivize users to add liquidity where it is needed. Paid from the accumulated fee surplus.

### Target Liquidity
The ideal amount of underlying tokens that should be held on each chain. Set automatically by the Fee Manager to equal the average liquidity across all chains.

### Fee Surplus
The accumulated fees from unwraps, used to pay wrap rewards. When the surplus is depleted, wrap rewards are reduced.

### Cross-Chain Unwrap
Unwrapping zERC20 on a different chain from where you hold it, typically to access better liquidity and lower fees. The process bridges your zERC20, unwraps on the destination chain, and bridges the underlying back.

### Incentive Curve
The mathematical formula that determines fee and reward rates based on current liquidity relative to target. Uses a linear density function that increases as liquidity drops below target.

## Technical Terms

### Merkle Tree
A data structure used to efficiently verify that a specific piece of data (like a burn transaction) is part of a larger set. zERC20 uses Poseidon hash-based Merkle trees.

### Poseidon Hash
A hash function optimized for use in zero-knowledge circuits. Used in zERC20's Merkle trees for efficient proof generation.

### Nova
A folding scheme used for incrementally verifiable computation. zERC20 uses Nova for batch withdrawal proofs.

### Groth16
A zero-knowledge proof system used for single withdrawals in zERC20. Known for its small proof size and fast verification.

### Hash Chain
A sequence of hashes where each hash includes the previous hash. zERC20 uses a SHA-256 hash chain to ensure deterministic ordering of all transfers.

## Infrastructure Terms

### LayerZero
A cross-chain messaging protocol that enables zERC20's crosschain functionality. LayerZero relays Merkle roots between chains.

### Hub
The central contract that aggregates Merkle roots from all chains into a global root, enabling cross-chain withdrawals.

### Verifier
A per-chain contract that validates zero-knowledge proofs and authorizes withdrawals.

### Indexer
An off-chain service that tracks all zERC20 transfers, maintains Merkle trees, and provides data for proof generation.

### ICP (Internet Computer)
A blockchain platform hosting zERC20's stealth messaging infrastructure. ICP canisters store encrypted burn address information.

### VetKD
Verifiable encrypted threshold key derivation. Used by ICP canisters to enable encrypted messaging between senders and recipients.

## User-Facing Terms

### Invoice
A request for payment that includes a burn address. Recipients generate invoices and share them with payers.

### Scan
The process of checking for incoming private transfers by decrypting announcements from the ICP storage canister.

### Batch Withdrawal
Combining multiple incoming transfers into a single withdrawal transaction. Only the total amount is exposed on-chain.

### Partial Withdrawal
Withdrawing less than the full amount received. Useful for obscuring transfer amounts.

## Privacy Terms

### Sender-Recipient Unlinkability
The property that on-chain observers cannot determine the relationship between a sender's address and a recipient's withdrawal address.

### Amount Fingerprinting
The risk that unusual or unique transfer amounts could be used to correlate deposits and withdrawals.

### Anonymity Set
The set of possible senders or recipients for a given transaction. Larger anonymity sets provide stronger privacy.
