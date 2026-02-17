# End-to-End Walkthrough

This walkthrough deploys a **zUSDT** token on Arbitrum Sepolia and Optimism Sepolia with a Hub on Base Sepolia, then tests wrap, private send, and receive.

## Scenario

| Parameter | Value |
|-----------|-------|
| Token | zUSDT (6 decimals, backed by testnet USDT) |
| Hub Chain | Base Sepolia (chain ID `84532`, EID `40245`) |
| Token Chain A | Arbitrum Sepolia (chain ID `421614`, EID `40231`) |
| Token Chain B | Optimism Sepolia (chain ID `11155420`, EID `40232`) |
| LayerZero Endpoint | `0x6EDCE65403992e310A62460808c4b910D972f10f` (testnet) |

---

## Step 1: Deploy Contracts

Condensed commands (refer to [Contract Deployment](contracts.md) for details):

```bash
cd contracts
export PRIVATE_KEY=0x...
export DEPLOY_SALT=zUSDT
export HUB_EID=40245
export TOKEN_NAME=zUSDT
export TOKEN_SYMBOL=zUSDT
export TOKEN_DECIMALS=6

# 1. Hub on Base Sepolia
forge script script/DeployHub.s.sol:DeployHub --rpc-url $BASE_RPC --broadcast -vvvv

# 2. Verifier + Token on each chain
forge script script/DeployVerifierAndToken.s.sol:DeployVerifierAndToken --rpc-url $ARB_RPC --broadcast -vvvv
forge script script/DeployVerifierAndToken.s.sol:DeployVerifierAndToken --rpc-url $OP_RPC --broadcast -vvvv

# 3. LiquidityManager + Adaptor on each chain
export ZERC20=0x<token_proxy_address>
forge script script/DeployLiquidity.s.sol:DeployLiquidity --rpc-url $ARB_RPC --broadcast -vvvv
forge script script/DeployLiquidity.s.sol:DeployLiquidity --rpc-url $OP_RPC --broadcast -vvvv

# 4. Wire peers
# ... (see Contract Deployment page)

# 5. Register tokens on Hub
cast send $HUB_ADDRESS "registerToken((uint64,uint32,address,address))" \
  "($ARB_CHAIN_ID,$ARB_EID,$ARB_VERIFIER,$ARB_TOKEN)" \
  --rpc-url $BASE_RPC --private-key $PRIVATE_KEY

cast send $HUB_ADDRESS "registerToken((uint64,uint32,address,address))" \
  "($OP_CHAIN_ID,$OP_EID,$OP_VERIFIER,$OP_TOKEN)" \
  --rpc-url $BASE_RPC --private-key $PRIVATE_KEY
```

---

## Step 2: Create tokens.json

Create a `tokens.json` file with the deployed contract addresses:

```json
{
  "tokens": [
    {
      "tokenName": "zUSDT",
      "tokenSymbol": "zUSDT",
      "decimals": 6,
      "hubChainId": 84532,
      "hubEid": 40245,
      "hubAddress": "0x<HUB_ADDRESS>",
      "chains": [
        {
          "chainId": 421614,
          "eid": 40231,
          "rpcUrls": ["https://arb-sepolia.g.alchemy.com/v2/YOUR_KEY"],
          "tokenAddress": "0x<ARB_TOKEN>",
          "verifierAddress": "0x<ARB_VERIFIER>",
          "liquidityManagerAddress": "0x<ARB_LIQUIDITY_MANAGER>",
          "adaptorAddress": "0x<ARB_ADAPTOR>",
          "underlyingTokenAddress": "0x<ARB_USDT>"
        },
        {
          "chainId": 11155420,
          "eid": 40232,
          "rpcUrls": ["https://opt-sepolia.g.alchemy.com/v2/YOUR_KEY"],
          "tokenAddress": "0x<OP_TOKEN>",
          "verifierAddress": "0x<OP_VERIFIER>",
          "liquidityManagerAddress": "0x<OP_LIQUIDITY_MANAGER>",
          "adaptorAddress": "0x<OP_ADAPTOR>",
          "underlyingTokenAddress": "0x<OP_USDT>"
        }
      ],
      "relayIntervalSecs": 300,
      "broadcastIntervalSecs": 600
    }
  ]
}
```

---

## Step 3: Download Circuit Artifacts

```bash
cd circuit-setup
cargo run -- download
```

Ensure `NOVA_ARTIFACTS_DIR` is set to the correct path (default: `../nova_artifacts`).

---

## Step 4: Start Infrastructure

```bash
# Start indexer + crosschain-job
docker compose up -d

# Start decider (on host)
docker compose -f docker-compose.decider.yml up -d
cd decider-prover && cargo run --release
```

See [Infrastructure Setup](infrastructure.md) for detailed configuration of each service.

---

## Step 5: Wrap Tokens

Using the SDK to wrap USDT into zUSDT:

```typescript
import {
  normalizeTokens,
  findTokenByChain,
  createProviderForToken,
  wrapWithLiquidityManager,
} from "zerc20-client-sdk";
import { createWalletClient, http } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { arbitrumSepolia } from "viem/chains";

// Load custom tokens from your tokens.json
const tokensFile = await import("./tokens.json");
const { tokens } = normalizeTokens(tokensFile);
const arbToken = findTokenByChain(tokens, 421614n);

const account = privateKeyToAccount("0x...");
const walletClient = createWalletClient({
  account,
  chain: arbitrumSepolia,
  transport: http(arbToken.rpcUrls[0]),
});
const publicClient = createProviderForToken(arbToken);

const result = await wrapWithLiquidityManager({
  walletClient,
  publicClient,
  liquidityManagerAddress: arbToken.liquidityManagerAddress!,
  zerc20TokenAddress: arbToken.tokenAddress,
  amount: 100_000_000n, // 100 USDT (6 decimals)
});
console.log("Wrap tx:", result.transactionHash);
```

---

## Step 6: Private Send

```typescript
import {
  createSdk,
  preparePrivateSend,
  submitPrivateSendAnnouncement,
  getSeedMessage,
} from "zerc20-client-sdk";
import { keccak256, toBytes } from "viem";
import { HttpAgent } from "@dfinity/agent";

const sdk = createSdk();
const agent = await HttpAgent.create({ host: "https://ic0.app" });
const stealthClient = sdk.createStealthClient({
  agent,
  storageCanisterId: "YOUR_STORAGE_CANISTER_ID",
  keyManagerCanisterId: "YOUR_KEY_MANAGER_CANISTER_ID",
});

// Derive seed (hash signature to 32 bytes)
const seedMessage = await getSeedMessage();
const seedSignature = await walletClient.signMessage({
  message: seedMessage,
  account,
});
const seedHex = keccak256(toBytes(seedSignature));

// Prepare and submit
const preparation = await preparePrivateSend({
  client: stealthClient,
  recipientAddress: "0xRecipient...",
  recipientChainId: 421614n,
  seedHex,
});

// Transfer zERC20 to burn address (using viem)
// ... ERC-20 transfer to preparation.burnAddress ...

const result = await submitPrivateSendAnnouncement({
  client: stealthClient,
  preparation,
});
console.log("Announcement submitted:", result.announcement);
```

---

## Step 7: Receive

```typescript
import {
  createAuthorizationPayload,
  requestVetKey,
  scanReceivings,
  collectRedeemContext,
} from "zerc20-client-sdk";
import { hexToBytes } from "viem";

// Authorize
const authPayload = await createAuthorizationPayload(
  stealthClient,
  recipientAddress,
);
const authSig = await walletClient.signMessage({
  message: authPayload.message,
  account: recipientAccount,
});

// Get VetKey
const vetKey = await requestVetKey(
  stealthClient,
  recipientAddress,
  authPayload,
  hexToBytes(authSig),
);

// Scan
const receivings = await scanReceivings({ client: stealthClient, vetKey });
console.log(`Found ${receivings.length} receivings`);

// Collect redeem context and generate proof (see Proof Generation page)
```

---

## Step 8: Verify

Confirm everything is working:

- **Check indexer status:**
  ```bash
  curl http://localhost:8080/status
  ```
- **Check on-chain balances:**
  ```bash
  cast call $ARB_TOKEN "balanceOf(address)(uint256)" $YOUR_ADDRESS --rpc-url $ARB_RPC
  ```
- **Verify transfer roots** are relayed across chains by inspecting the crosschain-job logs:
  ```bash
  docker compose logs -f crosschain-job
  ```

---

**See also:** [Overview](overview.md) | [Contract Deployment](contracts.md) | [Infrastructure Setup](infrastructure.md) | [SDK Quick Start](../sdk/quickstart.md)
