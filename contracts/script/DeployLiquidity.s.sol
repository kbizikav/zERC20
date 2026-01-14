// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {console2} from "forge-std/console2.sol";
import {Adaptor} from "../src/liquidity/Adaptor.sol";
import {LiquidityManager} from "../src/liquidity/LiquidityManager.sol";
import {IncentiveLib} from "../src/libraries/IncentiveLib.sol";
import {zERC20} from "../src/zERC20.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";

/// @notice Deploys LiquidityManager (upgradeable proxy) and optionally the Adaptor that unwraps + bridges through Stargate.
/// Required env:
/// - ZERC20 (address): zERC20 token address that the manager mints/burns.
/// - PRIVATE_KEY (uint256): Broadcaster private key.
/// Optional env:
/// - LIQUIDITY_UNDERLYING_TOKEN (address): ERC20 token held as liquidity (or set in chain-config).
/// - LIQUIDITY_TARGET (uint256): Target liquidity level used for rewards/fees (defaults to 1_000_000e6).
/// - LIQUIDITY_K (uint256): Incentive strength coefficient k for the fee curve, in basis points (1 = 0.01%; defaults to 0; set explicitly for rewards/fees).
/// - LIQUIDITY_OWNER (address): Admin/fee manager for the LiquidityManager (defaults to broadcaster).
/// - SET_LIQUIDITY_AS_MINTER (uint256): When non-zero, attempts to set the manager as zERC20 minter (defaults to 1).
/// - ADAPTOR_STARGATE (address): Stargate endpoint; when set, deploys the Adaptor wired to the manager (or set in chain-config).
/// - LZ_ENDPOINT (address): LayerZero endpoint used to validate lzCompose callers.
/// - CHAIN_CONFIG_PATH (string): Optional path to per-chain defaults (underlyingToken/stargate); falls back to `config/chain-config.json`.
contract DeployLiquidity is DeterministicDeployer {
    string internal constant DEFAULT_CHAIN_CONFIG_PATH = "config/chain-config.json";

    struct Config {
        address zerc20Token;
        address underlyingToken;
        IncentiveLib.FeeParams fee;
        address owner;
        address stargate;
        address lzEndpoint;
        bool setMinter;
    }

    struct ChainConfig {
        address underlyingToken;
        address stargate;
    }

    error Zerc20TokenRequired();
    error UnderlyingTokenRequired();
    error LiquidityTargetRequired();
    error LzEndpointRequired();

    function run() external {
        Config memory cfg = _loadConfig();
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address broadcaster = vm.addr(deployerKey);
        if (cfg.owner == address(0)) {
            cfg.owner = broadcaster;
        }

        vm.startBroadcast(deployerKey);
        bytes32 baseSalt = _loadBaseSalt();
        console2.log("Deploying LiquidityManager at block", block.number);

        LiquidityManager implementation = new LiquidityManager{salt: _deriveSalt(baseSalt, "LIQUIDITY_MANAGER_IMPL")}(
            cfg.underlyingToken, cfg.zerc20Token
        );
        bytes memory initData = abi.encodeCall(LiquidityManager.initialize, (cfg.fee, cfg.owner));
        LiquidityManager manager = LiquidityManager(
            payable(_deployProxyAndInit(baseSalt, "LIQUIDITY_MANAGER_PROXY", address(implementation), initData))
        );

        console2.log("LiquidityManager implementation deployed at", address(implementation));
        console2.log("LiquidityManager proxy deployed at", address(manager));
        console2.log("  owner set to", cfg.owner);
        console2.log("  underlying token", cfg.underlyingToken);
        console2.log("  zERC20 token", cfg.zerc20Token);
        console2.log("  target liquidity", cfg.fee.targetLiquidity);
        console2.log("  incentive coefficient k", cfg.fee.k);

        if (cfg.setMinter) {
            try zERC20(cfg.zerc20Token).setMinter(address(manager)) {
                console2.log("  manager set as token minter");
            } catch (bytes memory) {
                console2.log("  failed to set minter; ensure broadcaster owns the token");
            }
        }

        if (cfg.stargate != address(0)) {
            if (cfg.lzEndpoint == address(0)) revert LzEndpointRequired();
            Adaptor adaptorImplementation = new Adaptor{salt: _deriveSalt(baseSalt, "ADAPTOR_IMPL")}(
                address(manager), cfg.stargate, cfg.lzEndpoint
            );
            bytes memory adaptorInitData = abi.encodeCall(Adaptor.initialize, (cfg.owner));
            Adaptor adaptor = Adaptor(
                payable(_deployProxyAndInit(baseSalt, "ADAPTOR_PROXY", address(adaptorImplementation), adaptorInitData))
            );
            console2.log("Adaptor implementation deployed at", address(adaptorImplementation));
            console2.log("Adaptor proxy deployed at", address(adaptor));
            console2.log("  owner set to", cfg.owner);
            console2.log("  stargate", cfg.stargate);
            console2.log("  lz endpoint", cfg.lzEndpoint);
        } else {
            console2.log("Adaptor skipped (ADAPTOR_STARGATE not set)");
        }

        vm.stopBroadcast();
    }

    function _loadConfig() internal view returns (Config memory cfg) {
        ChainConfig memory chainCfg = _loadChainConfig();

        cfg.zerc20Token = vm.envAddress("ZERC20");
        cfg.underlyingToken = vm.envOr("LIQUIDITY_UNDERLYING_TOKEN", chainCfg.underlyingToken);
        cfg.fee.targetLiquidity = vm.envOr("LIQUIDITY_TARGET", uint256(1_000_000e6));
        cfg.fee.k = vm.envOr("LIQUIDITY_K", uint256(1_000));
        cfg.owner = vm.envOr("LIQUIDITY_OWNER", address(0));
        cfg.stargate = vm.envOr("ADAPTOR_STARGATE", chainCfg.stargate);
        if (cfg.stargate != address(0)) {
            cfg.lzEndpoint = vm.envAddress("LZ_ENDPOINT");
        }
        cfg.setMinter = vm.envOr("SET_LIQUIDITY_AS_MINTER", uint256(1)) != 0;

        if (cfg.zerc20Token == address(0)) revert Zerc20TokenRequired();
        if (cfg.underlyingToken == address(0)) revert UnderlyingTokenRequired();
        if (cfg.fee.targetLiquidity == 0) revert LiquidityTargetRequired();
    }

    function _loadChainConfig() internal view returns (ChainConfig memory chainCfg) {
        string memory path = vm.envOr("CHAIN_CONFIG_PATH", DEFAULT_CHAIN_CONFIG_PATH);

        string memory json;
        // reading local config files is intentional in scripts
        // forge-lint: disable-next-line(unsafe-cheatcode)
        try vm.readFile(path) returns (string memory data) {
            json = data;
        } catch (bytes memory) {
            return chainCfg;
        }

        string memory base = string.concat(".chains[\"", _toString(block.chainid), "\"]");
        chainCfg.underlyingToken = _parseAddress(json, string.concat(base, ".underlyingToken"));
        chainCfg.stargate = _parseAddress(json, string.concat(base, ".stargate"));
    }

    function _parseAddress(string memory json, string memory key) private pure returns (address value) {
        try vm.parseJsonAddress(json, key) returns (address parsed) {
            value = parsed;
        } catch (bytes memory) {
            value = address(0);
        }
    }

    function _toString(uint256 value) internal pure returns (string memory) {
        if (value == 0) {
            return "0";
        }

        uint256 temp = value;
        uint256 digits;

        while (temp != 0) {
            digits++;
            temp /= 10;
        }

        bytes memory buffer = new bytes(digits);
        while (value != 0) {
            digits -= 1;
            buffer[digits] = bytes1(uint8(48 + (value % 10)));
            value /= 10;
        }

        return string(buffer);
    }
}
