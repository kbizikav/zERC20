// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {console2} from "forge-std/console2.sol";
import {Hub} from "../src/Hub.sol";
import {zERC20} from "../src/zERC20.sol";
import {Verifier} from "../src/Verifier.sol";
import {RootNovaDecider} from "../src/verifiers/RootNovaDecider.sol";
import {WithdrawGlobalNovaDecider} from "../src/verifiers/WithdrawGlobalNovaDecider.sol";
import {WithdrawLocalNovaDecider} from "../src/verifiers/WithdrawLocalNovaDecider.sol";
import {WithdrawGlobalGroth16Verifier} from "../src/verifiers/WithdrawGlobalGroth16Verifier.sol";
import {WithdrawLocalGroth16Verifier} from "../src/verifiers/WithdrawLocalGroth16Verifier.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";

/// @notice Deploys the Hub contract to Base Sepolia (or any chain) using config supplied via environment variables.
/// Required env:
/// - HUB_EID (uint)            : LayerZero endpoint id for the local chain.
/// - HUB_ENDPOINT (address)    : LayerZero endpoint address on the local chain.
/// Optional env:
/// - HUB_DELEGATE (address)    : Account allowed to manage LayerZero config (defaults to broadcaster).
contract DeployHub is DeterministicDeployer {
    function run() external {
        address endpoint = vm.envAddress("HUB_ENDPOINT");
        address delegate = vm.envOr("HUB_DELEGATE", address(0));
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);
        bytes32 baseSalt = _loadBaseSalt();

        vm.startBroadcast(deployerKey);
        console2.log("Deploying Hub at block", block.number);
        if (delegate == address(0)) {
            delegate = deployer;
        }

        Hub hubImpl = new Hub{salt: _deriveSalt(baseSalt, "HUB_IMPL")}();
        bytes memory hubInit = abi.encodeCall(Hub.initialize, (endpoint, delegate));
        ERC1967Proxy proxy = new ERC1967Proxy{salt: _deriveSalt(baseSalt, "HUB_PROXY")}(address(hubImpl), hubInit);
        Hub hub = Hub(address(proxy));

        console2.log("Hub implementation deployed at", address(hubImpl));
        console2.log("Hub proxy deployed at", address(hub));
        console2.log("Hub owner set to", delegate);

        vm.stopBroadcast();
    }
}

/// @notice Deploys the zERC20 token and Verifier contracts to an L2.
/// - Loads deployment parameters from environment variables.
/// - Root/withdraw verifiers are deployed within this script, so no external addresses are required.
contract DeployVerifierAndToken is DeterministicDeployer {
    struct ChainConfig {
        string tokenName;
        string tokenSymbol;
        uint32 hubEid;
        address endpoint;
        address delegate; // optional
        address minter; // optional
        address owner; // optional
    }

    struct VerifierArgs {
        address token;
        uint32 hubEid;
        address endpoint;
        address delegate;
        address rootDecider;
        address withdrawGlobal;
        address withdrawLocal;
        address withdrawGlobalGroth16;
        address withdrawLocalGroth16;
    }

    struct VerifierDeps {
        address rootDecider;
        address withdrawGlobal;
        address withdrawLocal;
        address withdrawGlobalGroth16;
        address withdrawLocalGroth16;
    }

    /// @notice Environment-driven deployment. Reads all parameters from `vm.env*` calls.
    function run() external {
        ChainConfig memory cfg = _loadConfigFromEnv();
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        _deploy(cfg, deployerKey);
    }

    function _loadConfigFromEnv() private view returns (ChainConfig memory cfg) {
        cfg.tokenName = vm.envString("TOKEN_NAME");
        cfg.tokenSymbol = vm.envString("TOKEN_SYMBOL");
        cfg.hubEid = uint32(vm.envUint("HUB_EID"));
        cfg.endpoint = vm.envAddress("VERIFIER_ENDPOINT");
        cfg.delegate = vm.envOr("VERIFIER_DELEGATE", address(0));
        cfg.minter = vm.envOr("ZERC20_MINTER", address(0));
        cfg.owner = vm.envOr("TOKEN_OWNER", address(0));

        require(bytes(cfg.tokenName).length != 0, "tokenName missing");
        require(bytes(cfg.tokenSymbol).length != 0, "tokenSymbol missing");
        require(cfg.hubEid != 0, "hubEid missing");
        require(cfg.endpoint != address(0), "endpoint missing");
    }

    function _deploy(ChainConfig memory cfg, uint256 deployerKey) private {
        vm.startBroadcast(deployerKey);
        console2.log("Deploying Verifier and Token at block", block.number);

        address deployer = vm.addr(deployerKey);
        if (cfg.delegate == address(0)) {
            cfg.delegate = deployer;
        }
        if (cfg.minter == address(0)) {
            cfg.minter = deployer;
        }

        address owner = cfg.owner == address(0) ? deployer : cfg.owner;
        address delegate = cfg.delegate;
        address minter = cfg.minter;
        uint32 hubEid = cfg.hubEid;
        address endpoint = cfg.endpoint;
        bytes32 baseSalt = _loadBaseSalt();

        zERC20 tokenImpl = new zERC20{salt: _deriveSalt(baseSalt, "TOKEN_IMPL")}();
        bytes memory tokenInit = abi.encodeCall(zERC20.initialize, (cfg.tokenName, cfg.tokenSymbol, owner));
        ERC1967Proxy tokenProxy =
            new ERC1967Proxy{salt: _deriveSalt(baseSalt, "TOKEN_PROXY")}(address(tokenImpl), tokenInit);
        zERC20 token = zERC20(address(tokenProxy));
        console2.log("Token implementation deployed at", address(tokenImpl));
        console2.log("Token proxy deployed at", address(token));
        console2.log("  owner set to", owner);

        VerifierDeps memory deps;
        deps.rootDecider = _deployRootDecider(baseSalt);
        deps.withdrawGlobal = _deployWithdrawGlobalDecider(baseSalt);
        deps.withdrawLocal = _deployWithdrawLocalDecider(baseSalt);
        deps.withdrawGlobalGroth16 = _deployWithdrawGlobalGroth16(baseSalt);
        deps.withdrawLocalGroth16 = _deployWithdrawLocalGroth16(baseSalt);

        Verifier verifier = _deployVerifier(baseSalt, token, hubEid, endpoint, delegate, deps);

        token.setVerifier(address(verifier));
        console2.log("  verifier set to", address(verifier));

        token.setMinter(minter);
        console2.log("  minter set to", minter);

        vm.stopBroadcast();
    }

    function _deployRootDecider(bytes32 baseSalt) private returns (address rootDecider) {
        RootNovaDecider instance = new RootNovaDecider{salt: _deriveSalt(baseSalt, "ROOT_DECIDER")}();
        rootDecider = address(instance);
        console2.log("  RootDecider deployed at", rootDecider);
    }

    function _deployWithdrawGlobalDecider(bytes32 baseSalt) private returns (address withdrawGlobal) {
        WithdrawGlobalNovaDecider instance =
            new WithdrawGlobalNovaDecider{salt: _deriveSalt(baseSalt, "WITHDRAW_GLOBAL_DECIDER")}();
        withdrawGlobal = address(instance);
        console2.log("  WithdrawGlobalDecider deployed at", withdrawGlobal);
    }

    function _deployWithdrawLocalDecider(bytes32 baseSalt) private returns (address withdrawLocal) {
        WithdrawLocalNovaDecider instance =
            new WithdrawLocalNovaDecider{salt: _deriveSalt(baseSalt, "WITHDRAW_LOCAL_DECIDER")}();
        withdrawLocal = address(instance);
        console2.log("  WithdrawLocalDecider deployed at", withdrawLocal);
    }

    function _deployWithdrawGlobalGroth16(bytes32 baseSalt) private returns (address withdrawGlobalGroth16) {
        WithdrawGlobalGroth16Verifier instance =
            new WithdrawGlobalGroth16Verifier{salt: _deriveSalt(baseSalt, "WITHDRAW_GLOBAL_GROTH16")}();
        withdrawGlobalGroth16 = address(instance);
        console2.log("  WithdrawGlobalGroth16Verifier deployed at", withdrawGlobalGroth16);
    }

    function _deployWithdrawLocalGroth16(bytes32 baseSalt) private returns (address withdrawLocalGroth16) {
        WithdrawLocalGroth16Verifier instance =
            new WithdrawLocalGroth16Verifier{salt: _deriveSalt(baseSalt, "WITHDRAW_LOCAL_GROTH16")}();
        withdrawLocalGroth16 = address(instance);
        console2.log("  WithdrawLocalGroth16Verifier deployed at", withdrawLocalGroth16);
    }

    function _deployVerifier(
        bytes32 baseSalt,
        zERC20 token,
        uint32 hubEid,
        address endpoint,
        address delegate,
        VerifierDeps memory deps
    ) private returns (Verifier verifier) {
        Verifier verifierImpl = new Verifier{salt: _deriveSalt(baseSalt, "VERIFIER_IMPL")}();
        VerifierArgs memory args = VerifierArgs({
            token: address(token),
            hubEid: hubEid,
            endpoint: endpoint,
            delegate: delegate,
            rootDecider: deps.rootDecider,
            withdrawGlobal: deps.withdrawGlobal,
            withdrawLocal: deps.withdrawLocal,
            withdrawGlobalGroth16: deps.withdrawGlobalGroth16,
            withdrawLocalGroth16: deps.withdrawLocalGroth16
        });
        bytes memory verifierInit = _encodeVerifierInit(args);
        ERC1967Proxy verifierProxy =
            new ERC1967Proxy{salt: _deriveSalt(baseSalt, "VERIFIER_PROXY")}(address(verifierImpl), verifierInit);
        verifier = Verifier(address(verifierProxy));

        console2.log("Verifier implementation deployed at", address(verifierImpl));
        console2.log("Verifier proxy deployed at", address(verifier));
        console2.log("Verifier owner set to", delegate);
    }

    function _encodeVerifierInit(VerifierArgs memory args) private pure returns (bytes memory) {
        return abi.encodeCall(
            Verifier.initialize,
            (
                args.token,
                args.hubEid,
                args.endpoint,
                args.delegate,
                args.rootDecider,
                args.withdrawGlobal,
                args.withdrawLocal,
                args.withdrawGlobalGroth16,
                args.withdrawLocalGroth16
            )
        );
    }
}
