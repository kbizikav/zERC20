// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.33;

/* solhint-disable gas-custom-errors */

import {console2} from "forge-std/console2.sol";
import {zERC20} from "../src/zERC20.sol";
import {Verifier} from "../src/Verifier.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";

/// @notice Resume script for DeployVerifierAndToken on Ethereum Mainnet.
/// @dev This script continues the deployment from where it left off.
///      The following contracts were already deployed successfully:
///      - zERC20 Implementation: 0x55F2c7F6e11BaA2C4Fb144B9142E01B3F088AFeb
///      - Token Proxy (zETH): 0x66639EF03D81A71e19728aF59fDF5ef9bCb7b480
///      - RootNovaDecider: 0x8b634B274406a21D1B8d88aC48181Be362e01e71
///      - WithdrawGlobalNovaDecider: 0x653AE0D812DFA159E6EDcFB2B7607Ff2316b68AC
///      - WithdrawLocalNovaDecider: 0x61c0FA0BEB5c6F81EFB5EE0838b1dA7F651d4890
///      - WithdrawGlobalGroth16Verifier: 0xD661be9c980C5604D6eB1252242153560d8B9EF6
///      - WithdrawLocalGroth16Verifier: 0xd301D8D9d4307BAc04543F8d61aa75D633dd2ddB
///      - Verifier Implementation: 0x574b3823bB5d3b0A34301196b30a173B62Ffe679
///
///      This script will:
///      1. Deploy the Verifier Proxy using CREATE3
///      2. Call token.setVerifier() to link the verifier
contract ResumeVerifierAndToken is DeterministicDeployer {
    // Already deployed contract addresses (hardcoded for this specific resume)
    address internal constant TOKEN_PROXY = 0x66639EF03D81A71e19728aF59fDF5ef9bCb7b480;
    address internal constant ROOT_DECIDER = 0x8b634B274406a21D1B8d88aC48181Be362e01e71;
    address internal constant WITHDRAW_GLOBAL_DECIDER = 0x653AE0D812DFA159E6EDcFB2B7607Ff2316b68AC;
    address internal constant WITHDRAW_LOCAL_DECIDER = 0x61c0FA0BEB5c6F81EFB5EE0838b1dA7F651d4890;
    address internal constant WITHDRAW_GLOBAL_GROTH16 = 0xD661be9c980C5604D6eB1252242153560d8B9EF6;
    address internal constant WITHDRAW_LOCAL_GROTH16 = 0xd301D8D9d4307BAc04543F8d61aa75D633dd2ddB;
    address internal constant VERIFIER_IMPL = 0x574b3823bB5d3b0A34301196b30a173B62Ffe679;

    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);

        // Load the same salt used in the original deployment
        bytes32 baseSalt = _loadBaseSalt();

        // Load HUB_EID from environment (same as original deployment)
        uint32 hubEid = uint32(vm.envUint("HUB_EID"));
        require(hubEid != 0, "HUB_EID missing");

        // Delegate defaults to deployer if not specified
        address delegate = vm.envOr("VERIFIER_DELEGATE", deployer);

        vm.startBroadcast(deployerKey);
        console2.log("Resuming Verifier and Token deployment at block", block.number);
        console2.log("Using baseSalt:", vm.toString(baseSalt));
        console2.log("HUB_EID:", hubEid);

        // Deploy Verifier Proxy with initialization
        Verifier verifier = _deployVerifierProxy(deployer, baseSalt, hubEid, delegate);

        // Set verifier on token
        zERC20 token = zERC20(TOKEN_PROXY);
        token.setVerifier(address(verifier));
        console2.log("Token verifier set to", address(verifier));

        vm.stopBroadcast();

        console2.log("");
        console2.log("=== Deployment Complete ===");
        console2.log("Token Proxy:", TOKEN_PROXY);
        console2.log("Verifier Proxy:", address(verifier));
    }

    function _deployVerifierProxy(address deployer, bytes32 baseSalt, uint32 hubEid, address delegate)
        private
        returns (Verifier verifier)
    {
        bytes memory verifierInit = abi.encodeCall(
            Verifier.initialize,
            (
                TOKEN_PROXY,
                hubEid,
                delegate,
                ROOT_DECIDER,
                WITHDRAW_GLOBAL_DECIDER,
                WITHDRAW_LOCAL_DECIDER,
                WITHDRAW_GLOBAL_GROTH16,
                WITHDRAW_LOCAL_GROTH16,
                abi.encode("Verifier", "1")
            )
        );

        address verifierProxy = _deployProxyAndInit(deployer, baseSalt, "VERIFIER_PROXY", VERIFIER_IMPL, verifierInit);
        verifier = Verifier(verifierProxy);

        console2.log("Verifier proxy deployed at", address(verifier));
        console2.log("  owner set to", delegate);
    }
}
