# Wrap and Unwrap

The `LiquidityManager` contract converts between underlying tokens (USDC, ETH, BNB) and their zERC20 counterparts (zUSDC, zETH, zBNB). Wrapping deposits the underlying token and mints zERC20 -- the protocol adds a **reward** incentive to encourage deposits. Unwrapping burns zERC20 and returns the underlying token minus a **fee**. Both reward and fee are driven by a target-liquidity curve that keeps pool balances healthy.

## EVM Providers

All wrap/unwrap functions use the library-agnostic `EvmWriteProvider` and `EvmReadProvider` interfaces. viem's `WalletClient` and `PublicClient` satisfy these interfaces directly -- no adapter needed.

```typescript
import type { EvmReadProvider, EvmWriteProvider } from "zerc20-client-sdk";
```

## Wrap

Wrap an underlying token into its zERC20 equivalent.

```typescript
function wrapWithLiquidityManager(
  params: WrapWithLiquidityManagerParams,
): Promise<LiquidityActionResult>;
```

### WrapWithLiquidityManagerParams

| Field | Type | Description |
|-------|------|-------------|
| `writeProvider` | `EvmWriteProvider` | Write provider used to sign and send transactions |
| `readProvider?` | `EvmReadProvider` | Optional read provider for contract reads and receipt polling |
| `liquidityManagerAddress` | `string` | Address of the LiquidityManager contract |
| `amount` | `bigint \| number \| string` | Amount of underlying token to wrap (in base units) |
| `underlyingTokenAddress?` | `string` | Override underlying token address (auto-read from contract if omitted) |
| `recipient?` | `string` | Recipient of the minted zERC20 (defaults to the sender) |
| `feeOverrides?` | `FeeOverrides` | Optional gas-price / gas-limit overrides |

For **native ETH wrapping**, the SDK sends `msg.value` automatically -- no ERC-20 approval is needed. For ERC-20 underlying tokens the SDK checks the current allowance and submits an approval transaction first if required.

### LiquidityActionResult

| Field | Type | Description |
|-------|------|-------------|
| `transactionHash` | `Hex` | Hash of the wrap (or unwrap) transaction |
| `approvalTransactionHash?` | `Hex` | Hash of the ERC-20 approval transaction, if one was sent |

### Quote

Before wrapping, you can query the on-chain `quoteWrapReward` to preview the reward:

```typescript
const reward = await readProvider.readContract({
  address: entry.liquidityManagerAddress,
  abi: LiquidityManagerArtifact.abi,
  functionName: "quoteWrapReward",
  args: [100_000_000n],
});
console.log("Reward:", reward);
```

> **Note:** `LiquidityManagerArtifact` is exported from the SDK as a contract ABI artifact.

### Example -- Wrap 100 USDC to zUSDC

```typescript
import {
  normalizeTokens,
  findTokenByChain,
  wrapWithLiquidityManager,
} from "zerc20-client-sdk";

// 1. Resolve addresses from the token registry
const tokensFile = await import("./tokens.json");
const { tokens } = normalizeTokens(tokensFile);
const entry = findTokenByChain(tokens, 42161n); // Arbitrum

// 2. Execute the wrap
const result = await wrapWithLiquidityManager({
  writeProvider,                                    // EvmWriteProvider
  readProvider,                                     // optional EvmReadProvider
  liquidityManagerAddress: entry.liquidityManagerAddress,
  amount: 100_000_000n, // 100 USDC (6 decimals)
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
| `writeProvider` | `EvmWriteProvider` | Write provider used to sign and send transactions |
| `readProvider?` | `EvmReadProvider` | Optional read provider for contract reads |
| `liquidityManagerAddress` | `string` | Address of the LiquidityManager contract |
| `zerc20TokenAddress` | `string` | Address of the zERC20 token to burn |
| `amount` | `bigint \| number \| string` | Amount of zERC20 to unwrap (in base units) |
| `recipient?` | `string` | Recipient of the underlying token (defaults to the sender) |
| `feeOverrides?` | `FeeOverrides` | Optional gas-price / gas-limit overrides |
| `minAmountOut?` | `bigint` | Minimum underlying to receive; a **pre-flight slippage check** is performed before submitting |

When `minAmountOut` is set the SDK simulates the unwrap off-chain first. If the expected output falls below the threshold, the call rejects immediately with a slippage error instead of submitting a transaction that would revert on-chain.

### Quote

```typescript
function quoteLocalUnwrap(
  params: { provider: EvmReadProvider; liquidityManagerAddress: string; amount: bigint | number | string },
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

Pass the `CrossUnwrapQuote` together with a `writeProvider` and the contract addresses. The SDK attaches `sendNativeFee` as `msg.value` automatically.

## Fee Estimation

Use the quote helpers to estimate costs **before** submitting a transaction:

| Operation | Helper | Key fields |
|-----------|--------|------------|
| Local wrap | `readProvider.readContract()` with `quoteWrapReward` | reward amount |
| Local unwrap | `quoteLocalUnwrap()` | `fee`, `expectedOut` |
| Cross-chain unwrap | `buildCrossUnwrapQuote()` | `tokenUnwrapFee`, `nativeBridgeFee`, `tokenBridgeFee`, `expectedOut` |
| Stuck fund check | `hasStuckFunds()` | `boolean` |
| Stuck fund balances | `fetchAdaptorBalances()` | `underlyingTokenBalance`, `zerc20Balance`, `nativeBalance` |

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
| `underlyingAddress` | `string` | Address of the underlying ERC-20 (or zero address for native ETH) |
| `underlyingBalance` | `bigint` | Current underlying token balance held by the pool |
| `underlyingDecimals` | `number` | Decimals of the underlying token |
| `zerc20Balance` | `bigint` | Current zERC20 balance held by the pool |
| `zerc20Decimals` | `number` | Decimals of the zERC20 token |

## Stuck Fund Recovery (Adaptor Withdraw)

When a cross-chain unwrap fails (e.g. due to Stargate liquidity shortage), user funds may remain in the destination chain's Adaptor contract. The SDK provides functions to detect and recover these stuck funds.

### Check for Stuck Funds

```typescript
import { hasStuckFunds } from "zerc20-client-sdk";

const stuck = await hasStuckFunds({
  provider: readProvider,
  account: "0xUser...",
  adaptorAddress: "0xAdaptor...",
});
```

### Fetch Detailed Balances

```typescript
import { fetchAdaptorBalances } from "zerc20-client-sdk";

const balances = await fetchAdaptorBalances({
  provider: readProvider,
  account: "0xUser...",
  adaptorAddress: "0xAdaptor...",
});
// balances.underlyingTokenBalance — underlying token stuck in adaptor
// balances.zerc20Balance          — zERC20 token stuck in adaptor
// balances.nativeBalance          — native token stuck in adaptor
// balances.underlyingTokenAddress — underlying token contract address
// balances.zerc20TokenAddress     — zERC20 token contract address
```

### Withdraw Stuck Funds

```typescript
import { withdrawFromAdaptor, NATIVE_TOKEN_ADDRESS } from "zerc20-client-sdk";

const result = await withdrawFromAdaptor({
  writeProvider,
  adaptorAddress: "0xAdaptor...",
  token: balances.underlyingTokenAddress, // or zerc20TokenAddress, or NATIVE_TOKEN_ADDRESS
  amount: balances.underlyingTokenBalance,
});
console.log("Withdraw tx:", result.transactionHash);
```

### AdaptorBalances

| Field | Type | Description |
|-------|------|-------------|
| `underlyingTokenBalance` | `bigint` | Underlying token balance stuck in the adaptor |
| `zerc20Balance` | `bigint` | zERC20 token balance stuck in the adaptor |
| `nativeBalance` | `bigint` | Native token balance stuck in the adaptor |
| `underlyingTokenAddress` | `` `0x${string}` `` | Underlying token address as configured in the adaptor |
| `zerc20TokenAddress` | `` `0x${string}` `` | zERC20 token address as configured in the adaptor |

### AdaptorWithdrawParams

| Field | Type | Description |
|-------|------|-------------|
| `writeProvider` | `EvmWriteProvider` | Write provider used to sign and submit transactions |
| `readProvider?` | `EvmReadProvider` | Optional read provider for receipt polling |
| `adaptorAddress` | `string` | Adaptor contract address |
| `token` | `string` | Token address to withdraw (underlying, zERC20, or `NATIVE_TOKEN_ADDRESS`) |
| `amount` | `bigint \| number \| string` | Amount to withdraw |
| `feeOverrides?` | `FeeOverrides` | Optional gas-price / gas-limit overrides |

## Error Handling

| Scenario | Behaviour |
|----------|-----------|
| **Under-collateralized pool** | The wrap reward may drop to zero or become negative (i.e., no bonus). The transaction still succeeds but the caller receives fewer zERC20 than the deposited amount. |
| **Slippage exceeded** | When `minAmountOut` is set and the simulated output is too low, the SDK throws `"Price changed beyond slippage tolerance"` without sending a transaction. |
| **Allowance rejection** | The SDK sends an ERC-20 `approve` transaction automatically before wrapping/unwrapping. If the wallet rejects the approval popup the entire operation fails. |
| **Cross-chain unwrap failure** | If the Stargate bridge leg fails (e.g. liquidity shortage), funds remain in the destination Adaptor contract. Use `hasStuckFunds()` to detect and `withdrawFromAdaptor()` to recover. |

## Next Steps

- [SDK Quick Start](quickstart.md) -- installation and first private send
- [Receiving](receiving.md) -- scanning and claiming incoming transfers
- [Proof Generation](proof-generation.md) -- generating ZKP proofs for teleport
