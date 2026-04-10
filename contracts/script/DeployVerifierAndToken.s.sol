// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.33;

/* solhint-disable gas-custom-errors */

import {console2} from "forge-std/console2.sol";
import {zERC20} from "../src/zERC20.sol";
import {IBlocklist} from "../src/interfaces/IBlocklist.sol";
import {Verifier} from "../src/Verifier.sol";
import {RootNovaDecider} from "../src/verifiers/RootNovaDecider.sol";
import {WithdrawGlobalNovaDecider} from "../src/verifiers/WithdrawGlobalNovaDecider.sol";
import {WithdrawLocalNovaDecider} from "../src/verifiers/WithdrawLocalNovaDecider.sol";
import {WithdrawGlobalGroth16Verifier} from "../src/verifiers/WithdrawGlobalGroth16Verifier.sol";
import {WithdrawLocalGroth16Verifier} from "../src/verifiers/WithdrawLocalGroth16Verifier.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";
import {LZAddressContext} from "lz-address-book/helpers/LZAddressContext.sol";

/// @notice Deploys the zERC20 token and Verifier contracts to an L2.
/// - Loads deployment parameters from environment variables and resolves the LayerZero endpoint via lz-address-book.
/// - Root/withdraw verifiers are deployed within this script, so no external addresses are required.
contract DeployVerifierAndToken is DeterministicDeployer {
    struct ChainConfig {
        string tokenName;
        string tokenSymbol;
        uint32 hubEid;
        address endpoint;
        address delegate; // optional
        address owner; // optional
        uint8 tokenDecimals;
        address blocklist;
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
        bytes eip712Init;
    }

    struct VerifierDeps {
        address rootDecider;
        address withdrawGlobal;
        address withdrawLocal;
        address withdrawGlobalGroth16;
        address withdrawLocalGroth16;
    }

    /// @notice Environment-driven deployment with LayerZero endpoint auto-resolved via lz-address-book.
    function run() external {
        ChainConfig memory cfg = _loadConfigFromEnv();
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        _deploy(cfg, deployerKey);
    }

    function _loadConfigFromEnv() private returns (ChainConfig memory cfg) {
        cfg.tokenName = vm.envString("TOKEN_NAME");
        cfg.tokenSymbol = vm.envString("TOKEN_SYMBOL");
        cfg.hubEid = uint32(vm.envUint("HUB_EID"));
        cfg.endpoint = _resolveLzEndpoint();
        cfg.delegate = vm.envOr("VERIFIER_DELEGATE", address(0));
        cfg.owner = vm.envOr("TOKEN_OWNER", address(0));
        uint256 decimals = vm.envOr("TOKEN_DECIMALS", uint256(18));
        require(decimals <= type(uint8).max, "tokenDecimals too large");
        require(decimals >= 6, "decimals must be >= 6");
        // casting to uint8 is safe because decimals is bounds-checked above
        // forge-lint: disable-next-line(unsafe-typecast)
        cfg.tokenDecimals = uint8(decimals);

        cfg.blocklist = vm.envAddress("BLOCKLIST_ADDRESS");

        require(bytes(cfg.tokenName).length != 0, "tokenName missing");
        require(bytes(cfg.tokenSymbol).length != 0, "tokenSymbol missing");
        require(cfg.hubEid != 0, "hubEid missing");
        require(cfg.endpoint != address(0), "endpoint missing");
        require(cfg.blocklist != address(0), "blocklist missing");
    }

    function _resolveLzEndpoint() private returns (address endpoint) {
        LZAddressContext ctx = new LZAddressContext();
        ctx.setChainByChainId(block.chainid);
        endpoint = ctx.getEndpointV2();
    }

    function _deploy(ChainConfig memory cfg, uint256 deployerKey) private {
        vm.startBroadcast(deployerKey);
        console2.log("Deploying Verifier and Token at block", block.number);

        address deployer = vm.addr(deployerKey);
        if (cfg.delegate == address(0)) {
            cfg.delegate = deployer;
        }

        address owner = cfg.owner == address(0) ? deployer : cfg.owner;
        address delegate = cfg.delegate;
        uint32 hubEid = cfg.hubEid;
        address endpoint = cfg.endpoint;
        bytes32 baseSalt = _loadBaseSalt();

        bytes memory tokenImplCode =
            abi.encodePacked(type(zERC20).creationCode, abi.encode(endpoint, cfg.tokenDecimals, cfg.blocklist));
        zERC20 tokenImpl = zERC20(_deploy3(deployer, baseSalt, "TOKEN_IMPL", tokenImplCode));
        bytes memory tokenInit = abi.encodeCall(zERC20.initialize, (cfg.tokenName, cfg.tokenSymbol, owner));
        zERC20 token = zERC20(_deployProxyAndInit(deployer, baseSalt, "TOKEN_PROXY", address(tokenImpl), tokenInit));
        console2.log("Token implementation deployed at", address(tokenImpl));
        console2.log("Token proxy deployed at", address(token));
        console2.log("  owner set to", owner);

        VerifierDeps memory deps;
        deps.rootDecider = _deployRootDecider(deployer);
        deps.withdrawGlobal = _deployWithdrawGlobalDecider(deployer);
        deps.withdrawLocal = _deployWithdrawLocalDecider(deployer);
        deps.withdrawGlobalGroth16 = _deployWithdrawGlobalGroth16(deployer);
        deps.withdrawLocalGroth16 = _deployWithdrawLocalGroth16(deployer);

        Verifier verifier = _deployVerifier(deployer, baseSalt, token, hubEid, endpoint, delegate, deps);

        token.setVerifier(address(verifier));
        console2.log("  verifier set to", address(verifier));

        vm.stopBroadcast();
    }

    function _deployRootDecider(address deployer) private returns (address rootDecider) {
        bytes memory code = type(RootNovaDecider).creationCode;
        rootDecider = _deploy3Global(deployer, "ROOT_DECIDER", code);
        console2.log("  RootDecider deployed at", rootDecider);
    }

    function _deployWithdrawGlobalDecider(address deployer) private returns (address withdrawGlobal) {
        bytes memory code = type(WithdrawGlobalNovaDecider).creationCode;
        withdrawGlobal = _deploy3Global(deployer, "WITHDRAW_GLOBAL_DECIDER", code);
        console2.log("  WithdrawGlobalDecider deployed at", withdrawGlobal);
    }

    function _deployWithdrawLocalDecider(address deployer) private returns (address withdrawLocal) {
        bytes memory code = type(WithdrawLocalNovaDecider).creationCode;
        withdrawLocal = _deploy3Global(deployer, "WITHDRAW_LOCAL_DECIDER", code);
        console2.log("  WithdrawLocalDecider deployed at", withdrawLocal);
    }

    function _deployWithdrawGlobalGroth16(address deployer) private returns (address withdrawGlobalGroth16) {
        bytes memory code = type(WithdrawGlobalGroth16Verifier).creationCode;
        withdrawGlobalGroth16 = _deploy3Global(deployer, "WITHDRAW_GLOBAL_GROTH16", code);
        console2.log("  WithdrawGlobalGroth16Verifier deployed at", withdrawGlobalGroth16);
    }

    function _deployWithdrawLocalGroth16(address deployer) private returns (address withdrawLocalGroth16) {
        bytes memory code = type(WithdrawLocalGroth16Verifier).creationCode;
        withdrawLocalGroth16 = _deploy3Global(deployer, "WITHDRAW_LOCAL_GROTH16", code);
        console2.log("  WithdrawLocalGroth16Verifier deployed at", withdrawLocalGroth16);
    }

    function _deployVerifier(
        address deployer,
        bytes32 baseSalt,
        zERC20 token,
        uint32 hubEid,
        address endpoint,
        address delegate,
        VerifierDeps memory deps
    ) private returns (Verifier verifier) {
        bytes memory verifierImplCode = abi.encodePacked(type(Verifier).creationCode, abi.encode(endpoint));
        Verifier verifierImpl = Verifier(_deploy3(deployer, baseSalt, "VERIFIER_IMPL", verifierImplCode));
        VerifierArgs memory args = VerifierArgs({
            token: address(token),
            hubEid: hubEid,
            delegate: delegate,
            rootDecider: deps.rootDecider,
            withdrawGlobal: deps.withdrawGlobal,
            withdrawLocal: deps.withdrawLocal,
            withdrawGlobalGroth16: deps.withdrawGlobalGroth16,
            withdrawLocalGroth16: deps.withdrawLocalGroth16,
            eip712Init: abi.encode("Verifier", "1")
        });
        bytes memory verifierInit = _encodeVerifierInit(args);
        verifier =
            Verifier(_deployProxyAndInit(deployer, baseSalt, "VERIFIER_PROXY", address(verifierImpl), verifierInit));

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
                args.delegate,
                args.rootDecider,
                args.withdrawGlobal,
                args.withdrawLocal,
                args.withdrawGlobalGroth16,
                args.withdrawLocalGroth16,
                args.eip712Init
            )
        );
    }
}
