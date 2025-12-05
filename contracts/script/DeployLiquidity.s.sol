// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {console2} from "forge-std/console2.sol";
import {Adaptor} from "../src/liquidity/Adaptor.sol";
import {LiquidityManager} from "../src/liquidity/LiquidityManager.sol";
import {FeeLib} from "../src/libraries/FeeLib.sol";
import {zERC20} from "../src/zERC20.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";

/// @notice Deploys LiquidityManager (upgradeable proxy) and optionally the Adaptor that unwraps + bridges through Stargate.
/// Required env:
/// - ZERC20 (address): zERC20 token address that the manager mints/burns.
/// - PRIVATE_KEY (uint256): Broadcaster private key.
/// Optional env:
/// - LIQUIDITY_UNDERLYING_TOKEN (address): ERC20 token held as liquidity (or set in chain-config).
/// - LIQUIDITY_TARGET (uint256): Target liquidity level used for rewards/fees (defaults to 1_000_000e6).
/// - LIQUIDITY_OWNER (address): Admin/fee manager for the LiquidityManager (defaults to broadcaster).
/// - LIQUIDITY_REWARD_SLOPE_BPS (uint256): Reward slope in bps (defaults to 100).
/// - LIQUIDITY_FEE_LAMBDA1_BPS (uint256): Fee curve λ1 in bps (defaults to 40).
/// - LIQUIDITY_FEE_LAMBDA2_BPS (uint256): Fee curve λ2 in bps (defaults to 9_954).
/// - LIQUIDITY_FEE_DELTA1_BPS (uint256): Fee curve δ1 in bps of target (defaults to 6_000).
/// - LIQUIDITY_FEE_DELTA2_BPS (uint256): Fee curve δ2 in bps of target (defaults to 500).
/// - SET_LIQUIDITY_AS_MINTER (uint256): When non-zero, attempts to set the manager as zERC20 minter (defaults to 1).
/// - ADAPTOR_STARGATE (address): Stargate endpoint; when set, deploys the Adaptor wired to the manager (or set in chain-config).
/// - CHAIN_CONFIG_PATH (string): Optional path to per-chain defaults (underlyingToken/stargate); falls back to `config/chain-config.json`.
contract DeployLiquidity is DeterministicDeployer {
    string internal constant DEFAULT_CHAIN_CONFIG_PATH = "config/chain-config.json";

    struct Config {
        address zerc20Token;
        address underlyingToken;
        uint256 target;
        address owner;
        address stargate;
        bool setMinter;
        FeeLib.RewardParams reward;
        FeeLib.FeeParams fee;
    }

    struct ChainConfig {
        address underlyingToken;
        address stargate;
    }

    error Zerc20TokenRequired();
    error UnderlyingTokenRequired();
    error LiquidityTargetRequired();

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

        LiquidityManager implementation = new LiquidityManager{salt: _deriveSalt(baseSalt, "LIQUIDITY_MANAGER_IMPL")}();
        bytes memory initData = abi.encodeCall(
            LiquidityManager.initialize,
            (cfg.underlyingToken, cfg.zerc20Token, cfg.target, cfg.reward, cfg.fee, cfg.owner)
        );
        LiquidityManager manager = LiquidityManager(
            _deployProxyAndInit(baseSalt, "LIQUIDITY_MANAGER_PROXY", address(implementation), initData)
        );

        console2.log("LiquidityManager implementation deployed at", address(implementation));
        console2.log("LiquidityManager proxy deployed at", address(manager));
        console2.log("  owner set to", cfg.owner);
        console2.log("  underlying token", cfg.underlyingToken);
        console2.log("  zERC20 token", cfg.zerc20Token);
        console2.log("  target liquidity", cfg.target);
        console2.log("  fee params lambda1/lambda2", cfg.fee.lambda1Bps, cfg.fee.lambda2Bps);
        console2.log("  fee params delta1/delta2", cfg.fee.delta1Bps, cfg.fee.delta2Bps);
        console2.log("  reward slope bps", cfg.reward.liquiditySlopeBps);

        if (cfg.setMinter) {
            try zERC20(cfg.zerc20Token).setMinter(address(manager)) {
                console2.log("  manager set as token minter");
            } catch (bytes memory) {
                console2.log("  failed to set minter; ensure broadcaster owns the token");
            }
        }

        if (cfg.stargate != address(0)) {
            Adaptor adaptor = new Adaptor{salt: _deriveSalt(baseSalt, "ADAPTOR_IMPL")}(address(manager), cfg.stargate);
            console2.log("Adaptor deployed at", address(adaptor));
            console2.log("  stargate", cfg.stargate);
        } else {
            console2.log("Adaptor skipped (ADAPTOR_STARGATE not set)");
        }

        vm.stopBroadcast();
    }

    function _loadConfig() internal view returns (Config memory cfg) {
        ChainConfig memory chainCfg = _loadChainConfig();

        cfg.zerc20Token = vm.envAddress("ZERC20");
        cfg.underlyingToken = vm.envOr("LIQUIDITY_UNDERLYING_TOKEN", chainCfg.underlyingToken);
        cfg.target = vm.envOr("LIQUIDITY_TARGET", uint256(1_000_000e6));
        cfg.owner = vm.envOr("LIQUIDITY_OWNER", address(0));
        cfg.stargate = vm.envOr("ADAPTOR_STARGATE", chainCfg.stargate);
        cfg.setMinter = vm.envOr("SET_LIQUIDITY_AS_MINTER", uint256(1)) != 0;
        cfg.reward = FeeLib.RewardParams({liquiditySlopeBps: vm.envOr("LIQUIDITY_REWARD_SLOPE_BPS", uint256(100))});
        cfg.fee = FeeLib.FeeParams({
            lambda1Bps: vm.envOr("LIQUIDITY_FEE_LAMBDA1_BPS", uint256(40)),
            lambda2Bps: vm.envOr("LIQUIDITY_FEE_LAMBDA2_BPS", uint256(9_954)),
            delta1Bps: vm.envOr("LIQUIDITY_FEE_DELTA1_BPS", uint256(6_000)),
            delta2Bps: vm.envOr("LIQUIDITY_FEE_DELTA2_BPS", uint256(500))
        });

        if (cfg.zerc20Token == address(0)) revert Zerc20TokenRequired();
        if (cfg.underlyingToken == address(0)) revert UnderlyingTokenRequired();
        if (cfg.target == 0) revert LiquidityTargetRequired();
    }

    function _loadChainConfig() internal view returns (ChainConfig memory chainCfg) {
        string memory path = vm.envOr("CHAIN_CONFIG_PATH", DEFAULT_CHAIN_CONFIG_PATH);

        string memory json;
        try vm.readFile(path) returns (string memory data) {
            json = data;
        } catch (bytes memory) {
            return chainCfg;
        }

        string memory base = string.concat(".chains[\"", _toString(block.chainid), "\"]");
        chainCfg.underlyingToken = _parseAddress(json, string.concat(base, ".underlyingToken"));
        chainCfg.stargate = _parseAddress(json, string.concat(base, ".stargate"));
    }

    function _parseAddress(string memory json, string memory key) private view returns (address value) {
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
