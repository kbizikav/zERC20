# Deployed Contracts

This file tracks deployed contract addresses across environments.

> **Note**: Addresses here are for reference only. For programmatic use, prefer the
> `config/tokens*.json` files which include RPC URLs and other metadata.

## Network Reference

| Network | Chain ID | LayerZero EID | Role |
|---------|----------|---------------|------|
| Base Sepolia | 84532 | 40245 | Hub |
| Arbitrum Sepolia | 421614 | 40231 | Verifier/Token |
| Optimism Sepolia | 11155420 | 40232 | Verifier/Token |

## External Contracts (Testnet)

| Contract | Arbitrum Sepolia | Optimism Sepolia |
|----------|------------------|------------------|
| LayerZero Endpoint V2 | `0x6EDCE65403992e310A62460808c4b910D972f10f` | `0x6EDCE65403992e310A62460808c4b910D972f10f` |
| Stargate (ETH) | `0x6fddB6270F6c71f31B62AE0260cfa8E2e2d186E0` | `0xa31dCc5C71E25146b598bADA33E303627D7fC97e` |

## Deployer

| Environment | Address |
|-------------|---------|
| Testnet | `0x18DE9A6028cFAa0B4B58cc72E257b12e5625B396` |

---

## Testnet (Sepolia) - zUSD

USDC-backed token with 6 decimals.

### Hub (Base Sepolia)

| Contract | Address |
|----------|---------|
| Hub (proxy) | `0xA78CB62AD61025F2D7EF78348Cad89F31839513E` |

### Arbitrum Sepolia

| Contract | Address |
|----------|---------|
| zERC20 (proxy) | `0xbA240417D3843E71CaFE3780622624286a6F2Bd0` |
| Verifier (proxy) | `0x509F2aA410A3223a9B1eff2f06AAd40D754dB174` |
| LiquidityManager (proxy) | `0xb174fD86257ea56eEE75dFa5761C615Ce0E44Ff7` |
| Adaptor (proxy) | `0xe25817D3E7249F6a78fDe81d6eD316b75623C51E` |
| RootDecider | `0xf949AFB4a051449dd62283613E7B138A128CA8e8` |
| WithdrawGlobalDecider | `0xc7BB3f89F7D1791060209f61F7119ab74dd088fd` |
| WithdrawLocalDecider | `0xBA3f320F18f84f04382DdF69f3E3b2c0E242635a` |
| WithdrawGlobalGroth16Verifier | `0x0f478DB0f4D1E1dfE5656fb609CfD20495e0E45c` |
| WithdrawLocalGroth16Verifier | `0x662519d9D8050bCfff0C1c3b2eCA28cD762B1854` |

### Optimism Sepolia

| Contract | Address |
|----------|---------|
| zERC20 (proxy) | `0xbA240417D3843E71CaFE3780622624286a6F2Bd0` |
| Verifier (proxy) | `0x509F2aA410A3223a9B1eff2f06AAd40D754dB174` |
| LiquidityManager (proxy) | `0xE5EA4Ca667431F5120BeF9dD40E2cc4Aa3CE42D7` |
| Adaptor (proxy) | `0x65fe5855102d6C344e07363A0dcd721eAB14DC0B` |
| RootDecider | `0xf949AFB4a051449dd62283613E7B138A128CA8e8` |
| WithdrawGlobalDecider | `0xc7BB3f89F7D1791060209f61F7119ab74dd088fd` |
| WithdrawLocalDecider | `0xBA3f320F18f84f04382DdF69f3E3b2c0E242635a` |
| WithdrawGlobalGroth16Verifier | `0x0f478DB0f4D1E1dfE5656fb609CfD20495e0E45c` |
| WithdrawLocalGroth16Verifier | `0x662519d9D8050bCfff0C1c3b2eCA28cD762B1854` |

### Config File

See `config/tokens.zusdc.testnet.json` for the full zUSD configuration.

---

## Testnet (Sepolia) - zETH

Native ETH-backed token with 18 decimals. Uses `DEPLOY_SALT=zETH`.

### Hub (Base Sepolia)

Uses the same Hub as zUSD: `0xA78CB62AD61025F2D7EF78348Cad89F31839513E`

### Arbitrum Sepolia

Deployed at blocks: 10097166 (Verifier/Token), 10097248 (Liquidity/Adaptor)

| Contract | Address |
|----------|---------|
| zERC20 (proxy) | `0x64007Dd4818A530FDD3580341F02354e596772C6` |
| zERC20 (impl) | `0xC7f5b6F86d529Ac90db9BD30A3456147550171c0` |
| Verifier (proxy) | `0x0BD8923125B2c6A0093723f66D4B1EEa75aA0c5E` |
| Verifier (impl) | `0xD4564d92D1E2E681e0a7Ecf491d5E82aeD380F00` |
| LiquidityManager (proxy) | `0xBDE0a0929388865C6b6f883513e9bbe38CfBb46c` |
| LiquidityManager (impl) | `0x63affC83D7cEe21bCFb50822Dc0cAe265c5C4Ed7` |
| Adaptor (proxy) | `0x44Fa386c4b7a2F611A9c5BB057C1A65d3F42CcAf` |
| Adaptor (impl) | `0xA03e899B0a683c47506a34524ad5B885a7D9aEc1` |
| RootDecider | `0x6Ad826490E924AE1d9696f98e7D8231256039E39` |
| WithdrawGlobalDecider | `0x28904B39D9E8Cef6776E5F2B33b85cE551Ea0756` |
| WithdrawLocalDecider | `0x7f31c53a41040efA55d760C43cAACB620729F099` |
| WithdrawGlobalGroth16Verifier | `0xB7583FFa788f512e027c4c00FEAc7CA6d3e0DD59` |
| WithdrawLocalGroth16Verifier | `0x417d5FfEC916594CA8550723E768aBC69D8b0dcE` |

### Optimism Sepolia

Same addresses as Arbitrum Sepolia (deterministic CREATE3 deployment).
Deployed at blocks: 38631667 (Verifier/Token), 38632109 (Liquidity/Adaptor)

| Contract | Address |
|----------|---------|
| zERC20 (proxy) | `0x64007Dd4818A530FDD3580341F02354e596772C6` |
| zERC20 (impl) | `0xC7f5b6F86d529Ac90db9BD30A3456147550171c0` |
| Verifier (proxy) | `0x0BD8923125B2c6A0093723f66D4B1EEa75aA0c5E` |
| Verifier (impl) | `0xD4564d92D1E2E681e0a7Ecf491d5E82aeD380F00` |
| LiquidityManager (proxy) | `0xBDE0a0929388865C6b6f883513e9bbe38CfBb46c` |
| LiquidityManager (impl) | `0x63affC83D7cEe21bCFb50822Dc0cAe265c5C4Ed7` |
| Adaptor (proxy) | `0x44Fa386c4b7a2F611A9c5BB057C1A65d3F42CcAf` |
| Adaptor (impl) | `0xA03e899B0a683c47506a34524ad5B885a7D9aEc1` |
| RootDecider | `0x6Ad826490E924AE1d9696f98e7D8231256039E39` |
| WithdrawGlobalDecider | `0x28904B39D9E8Cef6776E5F2B33b85cE551Ea0756` |
| WithdrawLocalDecider | `0x7f31c53a41040efA55d760C43cAACB620729F099` |
| WithdrawGlobalGroth16Verifier | `0xB7583FFa788f512e027c4c00FEAc7CA6d3e0DD59` |
| WithdrawLocalGroth16Verifier | `0x417d5FfEC916594CA8550723E768aBC69D8b0dcE` |

### Config File

See `config/tokens.zeth.testnet.json` for the full zETH configuration.

---

## Mainnet - zUSDC

USDC-backed token with 6 decimals. Hub on Base.

**Hub**: `0xF52BEbC34195BA611891826120db435Ce6ADd9A1` (Base)

### Ethereum

| Contract | Address |
|----------|---------|
| zERC20 (proxy) | `0x9a0C876e626271fbD6D63cb46e3aFe532d77C320` |
| Verifier (proxy) | `0xD213aE536104b96421C31750cb802a24718ff8f3` |
| LiquidityManager (proxy) | `0x5f0520c7a9a137D17FA06E444c54499B6aE17Bfd` |
| Adaptor (proxy) | `0x4f0A72854e4bA79eAC69266AE9b2f934653371F1` |

### Arbitrum

| Contract | Address |
|----------|---------|
| zERC20 (proxy) | `0x9a0C876e626271fbD6D63cb46e3aFe532d77C320` |
| Verifier (proxy) | `0xD213aE536104b96421C31750cb802a24718ff8f3` |
| LiquidityManager (proxy) | `0x5f0520c7a9a137D17FA06E444c54499B6aE17Bfd` |
| Adaptor (proxy) | `0x4f0A72854e4bA79eAC69266AE9b2f934653371F1` |

### Base

| Contract | Address |
|----------|---------|
| zERC20 (proxy) | `0x9a0C876e626271fbD6D63cb46e3aFe532d77C320` |
| Verifier (proxy) | `0xD213aE536104b96421C31750cb802a24718ff8f3` |
| LiquidityManager (proxy) | `0x5f0520c7a9a137D17FA06E444c54499B6aE17Bfd` |
| Adaptor (proxy) | `0x4f0A72854e4bA79eAC69266AE9b2f934653371F1` |

### Config File

See `config/deployed/mainnet/tokens.zusdc.mainnet.json` for the full zUSDC configuration.

---

## Mainnet - zETH

Native ETH-backed token with 18 decimals. Hub on Base.

**Hub**: `0x58357afA19BF7ed2185C82508BBb03AaD1B2EfEe` (Base)

### Ethereum

| Contract | Address |
|----------|---------|
| zERC20 (proxy) | `0xff335e658286704251A158Ca7513d3e0B4074faa` |
| Verifier (proxy) | `0xc6c88CABDd0167B008f50ba60D4c7F1c434a7e1c` |
| LiquidityManager (proxy) | `0x85a19c3FF534EB00D268966CF6B50e7FF95aA3D2` |
| Adaptor (proxy) | `0x35955d908A4e431aD089816dBC42aAD089FCef21` |

### Arbitrum

| Contract | Address |
|----------|---------|
| zERC20 (proxy) | `0xff335e658286704251A158Ca7513d3e0B4074faa` |
| Verifier (proxy) | `0xc6c88CABDd0167B008f50ba60D4c7F1c434a7e1c` |
| LiquidityManager (proxy) | `0x85a19c3FF534EB00D268966CF6B50e7FF95aA3D2` |
| Adaptor (proxy) | `0x35955d908A4e431aD089816dBC42aAD089FCef21` |

### Base

| Contract | Address |
|----------|---------|
| zERC20 (proxy) | `0xff335e658286704251A158Ca7513d3e0B4074faa` |
| Verifier (proxy) | `0xc6c88CABDd0167B008f50ba60D4c7F1c434a7e1c` |
| LiquidityManager (proxy) | `0x85a19c3FF534EB00D268966CF6B50e7FF95aA3D2` |
| Adaptor (proxy) | `0x35955d908A4e431aD089816dBC42aAD089FCef21` |

### Config File

See `config/deployed/mainnet/tokens.zeth.mainnet.json` for the full zETH configuration.

---

## Mainnet - zBNB

Native BNB-backed token with 18 decimals. Hub on BNB Chain.

**Hub**: `0x675A949a8608483B828967EA984030fd6348418b` (BNB Chain)

### BNB Chain

| Contract | Address |
|----------|---------|
| zERC20 (proxy) | `0xB788bD0eDC23D36BED170665fa4D9B6A51Dc8488` |
| Verifier (proxy) | `0xFB56760c24E5E94c629171B462957f48149c8ACc` |
| LiquidityManager (proxy) | `0xACe7a7a22cD54Ee39827A35F9A3829AD2838eeDA` |
| Adaptor | — |

> **Note**: zBNB wrap/unwrap is only available on BNB Chain. Other chains support zBNB transfers via LayerZero OFT but do not have LiquidityManager contracts.

### Ethereum

| Contract | Address |
|----------|---------|
| zERC20 (proxy) | `0xB788bD0eDC23D36BED170665fa4D9B6A51Dc8488` |
| Verifier (proxy) | `0xFB56760c24E5E94c629171B462957f48149c8ACc` |
| LiquidityManager | — |
| Adaptor | — |

### Arbitrum

| Contract | Address |
|----------|---------|
| zERC20 (proxy) | `0xB788bD0eDC23D36BED170665fa4D9B6A51Dc8488` |
| Verifier (proxy) | `0xFB56760c24E5E94c629171B462957f48149c8ACc` |
| LiquidityManager | — |
| Adaptor | — |

### Base

| Contract | Address |
|----------|---------|
| zERC20 (proxy) | `0xB788bD0eDC23D36BED170665fa4D9B6A51Dc8488` |
| Verifier (proxy) | `0xFB56760c24E5E94c629171B462957f48149c8ACc` |
| LiquidityManager | — |
| Adaptor | — |

### Config File

See `config/deployed/mainnet/tokens.zbnb.mainnet.json` for the full zBNB configuration.

---

## Deployment Notes

### DEPLOY_SALT Values

| Token | DEPLOY_SALT | Notes |
|-------|-------------|-------|
| zUSD | (default) | Uses `keccak256("zerc20.deploy.default")` |
| zETH | `zETH` | Required to avoid address collision with zUSD |

### Underlying Token Addresses

| Token | Underlying | Sentinel/Address |
|-------|------------|------------------|
| zUSD | USDC | Per-chain USDC addresses (see `contracts/config/stargate/config.zUSDC.json`) |
| zETH | Native ETH | `0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE` (ERC-7528) |

### Verification Status

All testnet contracts have been verified on their respective block explorers.
