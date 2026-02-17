# Wrap and Unwrap

The `LiquidityManager` contract converts between underlying tokens (USDC, ETH, BNB) and their zERC20 counterparts (zUSDC, zETH, zBNB). Wrapping deposits the underlying token and mints zERC20 -- the protocol adds a **reward** incentive to encourage deposits. Unwrapping burns zERC20 and returns the underlying token minus a **fee**. Both reward and fee are driven by a target-liquidity curve that keeps pool balances healthy.

## Wrap

Wrap an underlying token into its zERC20 equivalent.

```typescript
function wrapWithLiquidityManager(
  params: LocalWrapParams,
): Promise<LiquidityActionResult>;
```

### LocalWrapParams

| Field | Type | Description |
|-------|------|-------------|
| `walletClient` | `WalletClient` | Wallet client used to sign and send transactions |
| `publicClient?` | `PublicClient` | Optional public client for read calls (derived from `walletClient` if omitted) |
| `liquidityManagerAddress` | `Address` | Address of the LiquidityManager contract |
| `zerc20TokenAddress` | `Address` | Address of the zERC20 token to receive |
| `amount` | `bigint \| number \| string` | Amount of underlying token to wrap (in base units) |
| `recipient?` | `Address` | Recipient of the minted zERC20 (defaults to the sender) |
| `feeOverrides?` | `FeeOverrides` | Optional gas-price / gas-limit overrides |
| `minAmountOut?` | `bigint` | Minimum zERC20 to receive; reverts on-chain if not met |

For **native ETH wrapping**, the SDK sends `msg.value` automatically -- no ERC-20 approval is needed. For ERC-20 underlying tokens the SDK checks the current allowance and submits an approval transaction first if required.

### LiquidityActionResult

| Field | Type | Description |
|-------|------|-------------|
| `transactionHash` | `Hex` | Hash of the wrap (or unwrap) transaction |
| `approvalTransactionHash?` | `Hex` | Hash of the ERC-20 approval transaction, if one was sent |

### Quote

Before wrapping, fetch an off-chain quote to preview the reward and expected output:

```typescript
function quoteLocalWrap(
  params: LocalWrapQuoteParams,
): Promise<LocalWrapQuote>;
```

`LocalWrapQuote` contains:

| Field | Type | Description |
|-------|------|-------------|
| `reward` | `bigint` | Bonus zERC20 minted on top of the deposited amount |
| `expectedOut` | `bigint` | Total zERC20 the caller will receive (`amount + reward`) |

### Example -- Wrap 100 USDC to zUSDC

```typescript
import {
  normalizeTokens,
  findTokenByChain,
  quoteLocalWrap,
  wrapWithLiquidityManager,
} from "zerc20-client-sdk";

// 1. Resolve addresses from the token registry
const tokensFile = await import("./tokens.json");
const { tokens } = normalizeTokens(tokensFile);
const entry = findTokenByChain(tokens, 42161n); // Arbitrum

// 2. Preview the wrap
const quote = await quoteLocalWrap({
  publicClient,
  liquidityManagerAddress: entry.liquidityManagerAddress,
  zerc20TokenAddress: entry.tokenAddress,
  amount: 100_000_000n, // 100 USDC (6 decimals)
});
console.log("Reward:", quote.reward);
console.log("Expected zUSDC out:", quote.expectedOut);

// 3. Execute the wrap
const result = await wrapWithLiquidityManager({
  walletClient,
  liquidityManagerAddress: entry.liquidityManagerAddress,
  zerc20TokenAddress: entry.tokenAddress,
  amount: 100_000_000n,
  minAmountOut: quote.expectedOut,
});
console.log("Wrap tx:", result.transactionHash);
if (result.approvalTransactionHash) {
  console.log("Approval tx:", result.approvalTransactionHash);
}
```

## Unwrap

Burn zERC20 and receive the underlying token.

```typescript
function unwrapWithLiquidityManager(
  params: LocalUnwrapParams,
): Promise<LiquidityActionResult>;
```

### LocalUnwrapParams

| Field | Type | Description |
|-------|------|-------------|
| `walletClient` | `WalletClient` | Wallet client used to sign and send transactions |
| `publicClient?` | `PublicClient` | Optional public client for read calls |
| `liquidityManagerAddress` | `Address` | Address of the LiquidityManager contract |
| `zerc20TokenAddress` | `Address` | Address of the zERC20 token to burn |
| `amount` | `bigint \| number \| string` | Amount of zERC20 to unwrap (in base units) |
| `recipient?` | `Address` | Recipient of the underlying token (defaults to the sender) |
| `feeOverrides?` | `FeeOverrides` | Optional gas-price / gas-limit overrides |
| `minAmountOut?` | `bigint` | Minimum underlying to receive; a **pre-flight slippage check** is performed before submitting |

When `minAmountOut` is set the SDK simulates the unwrap off-chain first. If the expected output falls below the threshold, the call rejects immediately with a slippage error instead of submitting a transaction that would revert on-chain.

### Quote

```typescript
function quoteLocalUnwrap(
  params: LocalUnwrapQuoteParams,
): Promise<LocalUnwrapQuote>;
```

`LocalUnwrapQuote` contains:

| Field | Type | Description |
|-------|------|-------------|
| `fee` | `bigint` | Fee deducted from the unwrapped amount |
| `expectedOut` | `bigint` | Underlying tokens the caller will receive (`amount - fee`) |

## Cross-Chain Unwrap

Unwrap zERC20 on the current chain and bridge the underlying token to a different chain via the Stargate bridge.

### Build a Quote

```typescript
function buildCrossUnwrapQuote(
  params: CrossUnwrapQuoteParams,
): Promise<CrossUnwrapQuote>;
```

`CrossUnwrapQuote` contains:

| Field | Type | Description |
|-------|------|-------------|
| `tokenUnwrapFee` | `bigint` | LiquidityManager unwrap fee (same as local unwrap) |
| `nativeBridgeFee` | `bigint` | Native gas fee required by Stargate / LayerZero |
| `tokenBridgeFee` | `bigint` | Token-denominated bridging fee |
| `sendNativeFee` | `bigint` | Total native value to attach to the transaction |
| `expectedOut` | `bigint` | Estimated underlying tokens received on the destination chain |
| `sendParam` | `SendParam` | Encoded Stargate send parameters |
| `bridgeRequest` | `BridgeRequest` | Full bridge request struct ready for submission |

### Execute

```typescript
function sendCrossUnwrap(
  params: CrossUnwrapSendParams,
): Promise<LiquidityActionResult>;
```

Pass the `CrossUnwrapQuote` together with a `walletClient` and the contract addresses. The SDK attaches `sendNativeFee` as `msg.value` automatically.

## Fee Estimation

Use the quote helpers to estimate costs **before** submitting a transaction:

| Operation | Helper | Key fields |
|-----------|--------|------------|
| Local wrap | `quoteLocalWrap()` | `reward`, `expectedOut` |
| Local unwrap | `quoteLocalUnwrap()` | `fee`, `expectedOut` |
| Cross-chain unwrap | `buildCrossUnwrapQuote()` | `tokenUnwrapFee`, `nativeBridgeFee`, `tokenBridgeFee`, `expectedOut` |

### Pool Balances

Inspect the current state of a LiquidityManager pool:

```typescript
function fetchLiquidityManagerBalances(
  params: FetchLiquidityBalancesParams,
): Promise<LiquidityBalances>;
```

`LiquidityBalances` contains:

| Field | Type | Description |
|-------|------|-------------|
| `underlyingAddress` | `Address` | Address of the underlying ERC-20 (or zero address for native ETH) |
| `underlyingBalance` | `bigint` | Current underlying token balance held by the pool |
| `underlyingDecimals` | `number` | Decimals of the underlying token |
| `zerc20Balance` | `bigint` | Current zERC20 balance held by the pool |
| `zerc20Decimals` | `number` | Decimals of the zERC20 token |

## Error Handling

| Scenario | Behaviour |
|----------|-----------|
| **Under-collateralized pool** | The wrap reward may drop to zero or become negative (i.e., no bonus). The transaction still succeeds but the caller receives fewer zERC20 than the deposited amount. |
| **Slippage exceeded** | When `minAmountOut` is set and the simulated output is too low, the SDK throws `"Price changed beyond slippage tolerance"` without sending a transaction. |
| **Allowance rejection** | The SDK sends an ERC-20 `approve` transaction automatically before wrapping/unwrapping. If the wallet rejects the approval popup the entire operation fails. |

## Next Steps

- [SDK Quick Start](quickstart.md) -- installation and first private send
- [Receiving](receiving.md) -- scanning and claiming incoming transfers
- [Proof Generation](proof-generation.md) -- generating ZKP proofs for teleport
