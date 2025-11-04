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
import {EndpointV2Mock, MockSendLib} from "../test/utils/TestHelperOz5.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";

/// @notice Local-only deployment script that bootstraps mock LayerZero endpoints, Hub, Verifier, and zERC20.
/// @dev Intended for Anvil or other local chains where real LayerZero endpoints are unavailable.
contract DeployLocal is DeterministicDeployer {
    struct Config {
        string tokenName;
        string tokenSymbol;
        uint32 hubEid;
        uint32 verifierEid;
        uint64 verifierChainId;
        address hubDelegate;
        address verifierDelegate;
        address minter;
        address tokenOwner;
        bool shareEndpoint;
        bool registerOnHub;
        bool wirePeers;
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

    function run() external {
        Config memory cfg = _loadConfig();
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);
        bytes32 baseSalt = _loadBaseSalt();

        vm.startBroadcast(deployerKey);

        EndpointV2Mock hubEndpoint = new EndpointV2Mock(cfg.hubEid);
        EndpointV2Mock verifierEndpoint = cfg.shareEndpoint ? hubEndpoint : new EndpointV2Mock(cfg.verifierEid);

        MockSendLib sendLib = new MockSendLib();
        address sendLibAddr = address(sendLib);
        _configureEndpoint(hubEndpoint, cfg.verifierEid, sendLibAddr);
        _configureEndpoint(verifierEndpoint, cfg.hubEid, sendLibAddr);

        Hub hub = _deployHub(cfg, deployer, hubEndpoint, baseSalt);
        zERC20 token = _deployToken(cfg, deployer, baseSalt);
        Verifier verifier = _deployVerifierSuite(cfg, deployer, verifierEndpoint, token, baseSalt);

        _finalizeDeployment(cfg, deployer, hub, token, verifier);

        vm.stopBroadcast();

        _logSummary(hubEndpoint, verifierEndpoint, hub, token, verifier);
    }

    function _loadConfig() private view returns (Config memory cfg) {
        cfg.tokenName = vm.envOr("TOKEN_NAME", string("Local zERC20"));
        cfg.tokenSymbol = vm.envOr("TOKEN_SYMBOL", string("LZERC"));
        cfg.hubEid = uint32(vm.envOr("HUB_EID", uint256(101)));
        cfg.verifierEid = uint32(vm.envOr("VERIFIER_EID", uint256(102)));
        cfg.verifierChainId = uint64(vm.envOr("VERIFIER_CHAIN_ID", uint256(block.chainid)));
        cfg.hubDelegate = vm.envOr("HUB_DELEGATE", address(0));
        cfg.verifierDelegate = vm.envOr("VERIFIER_DELEGATE", address(0));
        cfg.minter = vm.envOr("ZERC20_MINTER", address(0));
        cfg.tokenOwner = vm.envOr("TOKEN_OWNER", address(0));
        cfg.shareEndpoint = vm.envOr("SHARE_ENDPOINTS", uint256(0)) != 0;
        cfg.registerOnHub = vm.envOr("REGISTER_ON_HUB", uint256(1)) != 0;
        cfg.wirePeers = vm.envOr("WIRE_PEERS", uint256(1)) != 0;
    }

    function _configureEndpoint(EndpointV2Mock endpoint, uint32 dstEid, address lib) private {
        endpoint.setDefaultSendLibrary(dstEid, lib);
        endpoint.setDefaultReceiveLibrary(dstEid, lib, 0);
        endpoint.setMessagingFee(dstEid, 0, 0);
    }

    function _toBytes32(address addr) private pure returns (bytes32) {
        return bytes32(uint256(uint160(addr)));
    }

    function _deployHub(Config memory cfg, address deployer, EndpointV2Mock endpoint, bytes32 baseSalt)
        private
        returns (Hub hub)
    {
        address delegate = cfg.hubDelegate == address(0) ? deployer : cfg.hubDelegate;
        Hub impl = new Hub{salt: _deriveSalt(baseSalt, "HUB_IMPL")}();
        bytes memory initData = abi.encodeCall(Hub.initialize, (address(endpoint), delegate));
        ERC1967Proxy proxy = new ERC1967Proxy{salt: _deriveSalt(baseSalt, "HUB_PROXY")}(address(impl), initData);
        hub = Hub(address(proxy));
        console2.log("Hub implementation deployed at", address(impl));
        console2.log("Hub proxy deployed at", address(hub));
    }

    function _deployToken(Config memory cfg, address deployer, bytes32 baseSalt) private returns (zERC20 token) {
        address owner = cfg.tokenOwner == address(0) ? deployer : cfg.tokenOwner;
        zERC20 impl = new zERC20{salt: _deriveSalt(baseSalt, "TOKEN_IMPL")}();
        bytes memory initData = abi.encodeCall(zERC20.initialize, (cfg.tokenName, cfg.tokenSymbol, owner));
        ERC1967Proxy proxy =
            new ERC1967Proxy{salt: _deriveSalt(baseSalt, "TOKEN_PROXY")}(address(impl), initData);
        token = zERC20(address(proxy));
        console2.log("zERC20 implementation deployed at", address(impl));
        console2.log("zERC20 proxy deployed at", address(token));
        console2.log("Token owner set to", owner);
    }

    function _deployVerifierSuite(
        Config memory cfg,
        address deployer,
        EndpointV2Mock endpoint,
        zERC20 token,
        bytes32 baseSalt
    ) private returns (Verifier verifier) {
        VerifierDeps memory deps;
        deps.rootDecider = _deployRootDecider(baseSalt);
        deps.withdrawGlobal = _deployWithdrawGlobalDecider(baseSalt);
        deps.withdrawLocal = _deployWithdrawLocalDecider(baseSalt);
        deps.withdrawGlobalGroth16 = _deployWithdrawGlobalGroth16(baseSalt);
        deps.withdrawLocalGroth16 = _deployWithdrawLocalGroth16(baseSalt);

        address delegate = cfg.verifierDelegate == address(0) ? deployer : cfg.verifierDelegate;
        verifier = _deployVerifier(
            baseSalt,
            token,
            cfg.hubEid,
            address(endpoint),
            delegate,
            deps
        );

        token.setVerifier(address(verifier));
        console2.log("Token verifier wired");
    }

    function _deployRootDecider(bytes32 baseSalt) private returns (address rootDecider) {
        RootNovaDecider instance = new RootNovaDecider{salt: _deriveSalt(baseSalt, "ROOT_DECIDER")}();
        rootDecider = address(instance);
        console2.log("RootDecider deployed at", rootDecider);
    }

    function _deployWithdrawGlobalDecider(bytes32 baseSalt)
        private
        returns (address withdrawGlobal)
    {
        WithdrawGlobalNovaDecider instance =
            new WithdrawGlobalNovaDecider{salt: _deriveSalt(baseSalt, "WITHDRAW_GLOBAL_DECIDER")}();
        withdrawGlobal = address(instance);
        console2.log("WithdrawGlobalDecider deployed at", withdrawGlobal);
    }

    function _deployWithdrawLocalDecider(bytes32 baseSalt)
        private
        returns (address withdrawLocal)
    {
        WithdrawLocalNovaDecider instance =
            new WithdrawLocalNovaDecider{salt: _deriveSalt(baseSalt, "WITHDRAW_LOCAL_DECIDER")}();
        withdrawLocal = address(instance);
        console2.log("WithdrawLocalDecider deployed at", withdrawLocal);
    }

    function _deployWithdrawGlobalGroth16(bytes32 baseSalt)
        private
        returns (address withdrawGlobalGroth16)
    {
        WithdrawGlobalGroth16Verifier instance =
            new WithdrawGlobalGroth16Verifier{salt: _deriveSalt(baseSalt, "WITHDRAW_GLOBAL_GROTH16")}();
        withdrawGlobalGroth16 = address(instance);
        console2.log("WithdrawGlobalGroth16Verifier deployed at", withdrawGlobalGroth16);
    }

    function _deployWithdrawLocalGroth16(bytes32 baseSalt)
        private
        returns (address withdrawLocalGroth16)
    {
        WithdrawLocalGroth16Verifier instance =
            new WithdrawLocalGroth16Verifier{salt: _deriveSalt(baseSalt, "WITHDRAW_LOCAL_GROTH16")}();
        withdrawLocalGroth16 = address(instance);
        console2.log("WithdrawLocalGroth16Verifier deployed at", withdrawLocalGroth16);
    }

    function _deployVerifier(
        bytes32 baseSalt,
        zERC20 token,
        uint32 hubEid,
        address endpoint,
        address delegate,
        VerifierDeps memory deps
    ) private returns (Verifier verifier) {
        Verifier impl = new Verifier{salt: _deriveSalt(baseSalt, "VERIFIER_IMPL")}();
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
        bytes memory initData = _encodeVerifierInit(args);
        ERC1967Proxy proxy =
            new ERC1967Proxy{salt: _deriveSalt(baseSalt, "VERIFIER_PROXY")}(address(impl), initData);
        verifier = Verifier(address(proxy));
        console2.log("Verifier implementation deployed at", address(impl));
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

    function _finalizeDeployment(Config memory cfg, address deployer, Hub hub, zERC20 token, Verifier verifier)
        private
    {
        address minter = cfg.minter == address(0) ? deployer : cfg.minter;
        token.setMinter(minter);
        console2.log("Token minter set to", minter);

        if (cfg.wirePeers) {
            _wirePeers(cfg, hub, verifier);
        }

        if (cfg.registerOnHub) {
            _registerOnHub(cfg, hub, verifier, token);
        }
    }

    function _wirePeers(Config memory cfg, Hub hub, Verifier verifier) private {
        bytes32 verifierPeer = _toBytes32(address(verifier));
        hub.setPeer(cfg.verifierEid, verifierPeer);

        bytes32 hubPeer = _toBytes32(address(hub));
        verifier.setPeer(cfg.hubEid, hubPeer);

        console2.log("Peers configured for Hub and Verifier");
    }

    function _registerOnHub(Config memory cfg, Hub hub, Verifier verifier, zERC20 token) private {
        hub.registerToken(
            Hub.TokenInfo({
                chainId: cfg.verifierChainId,
                eid: cfg.verifierEid,
                verifier: address(verifier),
                token: address(token)
            })
        );
        console2.log("Token registered on Hub");
    }

    function _logSummary(
        EndpointV2Mock hubEndpoint,
        EndpointV2Mock verifierEndpoint,
        Hub hub,
        zERC20 token,
        Verifier verifier
    ) private pure {
        console2.log("Hub endpoint mock", address(hubEndpoint));
        console2.log("Verifier endpoint mock", address(verifierEndpoint));
        console2.log("Hub address", address(hub));
        console2.log("Token address", address(token));
        console2.log("Verifier address", address(verifier));
    }
}
