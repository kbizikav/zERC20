// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

/* solhint-disable gas-custom-errors */

import {console2} from "forge-std/console2.sol";
import {Hub} from "../src/Hub.sol";
import {zERC20} from "../src/zERC20.sol";
import {Verifier} from "../src/Verifier.sol";
import {RootNovaDecider} from "../src/verifiers/RootNovaDecider.sol";
import {WithdrawGlobalNovaDecider} from "../src/verifiers/WithdrawGlobalNovaDecider.sol";
import {WithdrawLocalNovaDecider} from "../src/verifiers/WithdrawLocalNovaDecider.sol";
import {WithdrawGlobalGroth16Verifier} from "../src/verifiers/WithdrawGlobalGroth16Verifier.sol";
import {WithdrawLocalGroth16Verifier} from "../src/verifiers/WithdrawLocalGroth16Verifier.sol";
import {EndpointV2Mock} from "@layerzerolabs/test-devtools-evm-foundry/contracts/mocks/EndpointV2Mock.sol";
import {SimpleMessageLibMock} from "@layerzerolabs/test-devtools-evm-foundry/contracts/mocks/SimpleMessageLibMock.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";

/// @dev Minimal helper used by SimpleMessageLibMock to satisfy schedulePacket hook.
contract NoopVerifyHelper {
    function schedulePacket(bytes calldata, bytes calldata) external {}
}

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
        address tokenOwner;
        uint8 tokenDecimals;
        bool shareEndpoint;
        bool registerOnHub;
        bool wirePeers;
    }

    struct VerifierArgs {
        address token;
        uint32 hubEid;
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

        (EndpointV2Mock hubEndpoint, EndpointV2Mock verifierEndpoint) = _deployEndpoints(cfg, deployer);

        Hub hub = _deployHub(cfg, deployer, hubEndpoint, baseSalt);
        zERC20 token = _deployToken(cfg, deployer, verifierEndpoint, baseSalt);
        Verifier verifier = _deployVerifierSuite(cfg, deployer, verifierEndpoint, token, baseSalt);

        _finalizeDeployment(cfg, deployer, hub, token, verifier);

        vm.stopBroadcast();

        _logSummary(hubEndpoint, verifierEndpoint, hub, token, verifier);
    }

    function _loadConfig() private view returns (Config memory cfg) {
        (cfg.tokenName, cfg.tokenSymbol, cfg.tokenDecimals, cfg.tokenOwner) = _loadTokenConfig();
        (cfg.hubEid, cfg.verifierEid, cfg.verifierChainId) = _loadNetworkConfig();
        (cfg.hubDelegate, cfg.verifierDelegate) = _loadDelegates();
        (cfg.shareEndpoint, cfg.registerOnHub, cfg.wirePeers) = _loadFlags();
    }

    function _loadTokenConfig()
        private
        view
        returns (string memory tokenName, string memory tokenSymbol, uint8 tokenDecimals, address tokenOwner)
    {
        tokenName = vm.envOr("TOKEN_NAME", string("Local zERC20"));
        tokenSymbol = vm.envOr("TOKEN_SYMBOL", string("LZERC"));
        tokenOwner = vm.envOr("TOKEN_OWNER", address(0));
        uint256 decimals = vm.envOr("TOKEN_DECIMALS", uint256(18));
        require(decimals <= type(uint8).max, "tokenDecimals too large");
        require(decimals >= 6, "decimals must be >= 6");
        // casting to uint8 is safe because decimals is bounds-checked above
        // forge-lint: disable-next-line(unsafe-typecast)
        tokenDecimals = uint8(decimals);
    }

    function _loadNetworkConfig() private view returns (uint32 hubEid, uint32 verifierEid, uint64 verifierChainId) {
        hubEid = uint32(vm.envOr("HUB_EID", uint256(101)));
        verifierEid = uint32(vm.envOr("VERIFIER_EID", uint256(102)));
        verifierChainId = uint64(vm.envOr("VERIFIER_CHAIN_ID", uint256(block.chainid)));
    }

    function _loadDelegates() private view returns (address hubDelegate, address verifierDelegate) {
        hubDelegate = vm.envOr("HUB_DELEGATE", address(0));
        verifierDelegate = vm.envOr("VERIFIER_DELEGATE", address(0));
    }

    function _loadFlags() private view returns (bool shareEndpoint, bool registerOnHub, bool wirePeers) {
        shareEndpoint = vm.envOr("SHARE_ENDPOINTS", uint256(0)) != 0;
        registerOnHub = vm.envOr("REGISTER_ON_HUB", uint256(1)) != 0;
        wirePeers = vm.envOr("WIRE_PEERS", uint256(1)) != 0;
    }

    function _deployEndpoints(Config memory cfg, address deployer)
        private
        returns (EndpointV2Mock hubEndpoint, EndpointV2Mock verifierEndpoint)
    {
        NoopVerifyHelper verifyHelper = new NoopVerifyHelper();
        hubEndpoint = new EndpointV2Mock(cfg.hubEid, deployer);
        verifierEndpoint = cfg.shareEndpoint ? hubEndpoint : new EndpointV2Mock(cfg.verifierEid, deployer);

        SimpleMessageLibMock hubSendLib = new SimpleMessageLibMock(payable(address(verifyHelper)), address(hubEndpoint));
        SimpleMessageLibMock verifierSendLib = cfg.shareEndpoint
            ? hubSendLib
            : new SimpleMessageLibMock(payable(address(verifyHelper)), address(verifierEndpoint));

        _configureEndpoint(hubEndpoint, cfg.verifierEid, hubSendLib);
        _configureEndpoint(verifierEndpoint, cfg.hubEid, verifierSendLib);
    }

    function _configureEndpoint(EndpointV2Mock endpoint, uint32 dstEid, SimpleMessageLibMock lib) private {
        endpoint.registerLibrary(address(lib));
        endpoint.setDefaultSendLibrary(dstEid, address(lib));
        endpoint.setDefaultReceiveLibrary(dstEid, address(lib), 0);
        lib.setMessagingFee(0, 0);
    }

    function _toBytes32(address addr) private pure returns (bytes32) {
        return bytes32(uint256(uint160(addr)));
    }

    function _deployHub(Config memory cfg, address deployer, EndpointV2Mock endpoint, bytes32 baseSalt)
        private
        returns (Hub hub)
    {
        address delegate = cfg.hubDelegate == address(0) ? deployer : cfg.hubDelegate;
        Hub impl = new Hub{salt: _deriveSalt(baseSalt, "HUB_IMPL")}(address(endpoint));
        bytes memory initData = abi.encodeCall(Hub.initialize, (delegate));
        hub = Hub(_deployProxyAndInit(baseSalt, "HUB_PROXY", address(impl), initData));
        console2.log("\tHub implementation deployed at", address(impl));
        console2.log("Hub proxy deployed at", address(hub));
    }

    function _deployToken(Config memory cfg, address deployer, EndpointV2Mock endpoint, bytes32 baseSalt)
        private
        returns (zERC20 token)
    {
        address owner = cfg.tokenOwner == address(0) ? deployer : cfg.tokenOwner;
        zERC20 impl = new zERC20{salt: _deriveSalt(baseSalt, "TOKEN_IMPL")}(address(endpoint), cfg.tokenDecimals);
        bytes memory initData = _encodeTokenInit(cfg.tokenName, cfg.tokenSymbol, owner);
        token = zERC20(_deployProxyAndInit(baseSalt, "TOKEN_PROXY", address(impl), initData));
        console2.log("\tzERC20 implementation deployed at", address(impl));
        console2.log("zERC20 proxy deployed at", address(token));
        console2.log("\tToken owner set to", owner);
    }

    function _encodeTokenInit(string memory name, string memory symbol, address owner)
        private
        pure
        returns (bytes memory)
    {
        return abi.encodeCall(zERC20.initialize, (name, symbol, owner));
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
        verifier = _deployVerifier(baseSalt, token, cfg.hubEid, address(endpoint), delegate, deps);

        token.setVerifier(address(verifier));
        console2.log("\tToken verifier wired");
    }

    function _deployRootDecider(bytes32 baseSalt) private returns (address rootDecider) {
        RootNovaDecider instance = new RootNovaDecider{salt: _deriveSalt(baseSalt, "ROOT_DECIDER")}();
        rootDecider = address(instance);
        console2.log("\tRootDecider deployed at", rootDecider);
    }

    function _deployWithdrawGlobalDecider(bytes32 baseSalt) private returns (address withdrawGlobal) {
        WithdrawGlobalNovaDecider instance =
            new WithdrawGlobalNovaDecider{salt: _deriveSalt(baseSalt, "WITHDRAW_GLOBAL_DECIDER")}();
        withdrawGlobal = address(instance);
        console2.log("\tWithdrawGlobalDecider deployed at", withdrawGlobal);
    }

    function _deployWithdrawLocalDecider(bytes32 baseSalt) private returns (address withdrawLocal) {
        WithdrawLocalNovaDecider instance =
            new WithdrawLocalNovaDecider{salt: _deriveSalt(baseSalt, "WITHDRAW_LOCAL_DECIDER")}();
        withdrawLocal = address(instance);
        console2.log("\tWithdrawLocalDecider deployed at", withdrawLocal);
    }

    function _deployWithdrawGlobalGroth16(bytes32 baseSalt) private returns (address withdrawGlobalGroth16) {
        WithdrawGlobalGroth16Verifier instance =
            new WithdrawGlobalGroth16Verifier{salt: _deriveSalt(baseSalt, "WITHDRAW_GLOBAL_GROTH16")}();
        withdrawGlobalGroth16 = address(instance);
        console2.log("\tWithdrawGlobalGroth16Verifier deployed at", withdrawGlobalGroth16);
    }

    function _deployWithdrawLocalGroth16(bytes32 baseSalt) private returns (address withdrawLocalGroth16) {
        WithdrawLocalGroth16Verifier instance =
            new WithdrawLocalGroth16Verifier{salt: _deriveSalt(baseSalt, "WITHDRAW_LOCAL_GROTH16")}();
        withdrawLocalGroth16 = address(instance);
        console2.log("\tWithdrawLocalGroth16Verifier deployed at", withdrawLocalGroth16);
    }

    function _deployVerifier(
        bytes32 baseSalt,
        zERC20 token,
        uint32 hubEid,
        address endpoint,
        address delegate,
        VerifierDeps memory deps
    ) private returns (Verifier verifier) {
        Verifier impl = new Verifier{salt: _deriveSalt(baseSalt, "VERIFIER_IMPL")}(endpoint);
        VerifierArgs memory args = VerifierArgs({
            token: address(token),
            hubEid: hubEid,
            delegate: delegate,
            rootDecider: deps.rootDecider,
            withdrawGlobal: deps.withdrawGlobal,
            withdrawLocal: deps.withdrawLocal,
            withdrawGlobalGroth16: deps.withdrawGlobalGroth16,
            withdrawLocalGroth16: deps.withdrawLocalGroth16
        });
        bytes memory initData = _encodeVerifierInit(args);
        verifier = Verifier(_deployProxyAndInit(baseSalt, "VERIFIER_PROXY", address(impl), initData));
        console2.log("\tVerifier implementation deployed at", address(impl));
        console2.log("Verifier proxy deployed at", address(verifier));
        console2.log("\tVerifier owner set to", delegate);
    }

    function _encodeVerifierInit(VerifierArgs memory args) private pure returns (bytes memory) {
        return abi.encodeCall(
            Verifier.initialize,
            (
                args.token,
                args.hubEid,
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
        token.setMinter(deployer);
        console2.log("\tToken minter set to deployer", deployer);

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

        console2.log("\tPeers configured for Hub and Verifier");
    }

    function _registerOnHub(Config memory cfg, Hub hub, Verifier verifier, zERC20 token) private {
        hub.registerToken(
            Hub.TokenInfo({
                chainId: cfg.verifierChainId, eid: cfg.verifierEid, verifier: address(verifier), token: address(token)
            })
        );
        console2.log("\tToken registered on Hub");
    }

    function _logSummary(
        EndpointV2Mock hubEndpoint,
        EndpointV2Mock verifierEndpoint,
        Hub hub,
        zERC20 token,
        Verifier verifier
    ) private pure {
        console2.log("\tHub endpoint mock", address(hubEndpoint));
        console2.log("\tVerifier endpoint mock", address(verifierEndpoint));
        console2.log("\tHub address", address(hub));
        console2.log("\tToken address", address(token));
        console2.log("\tVerifier address", address(verifier));
    }
}
