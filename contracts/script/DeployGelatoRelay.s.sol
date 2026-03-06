// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

/* solhint-disable gas-custom-errors */

import {console2} from "forge-std/console2.sol";
import {GelatoRelay} from "../src/relay/GelatoRelay.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";

/// @notice Deploys GelatoRelay via CREATE3 for deterministic cross-chain addresses.
/// Required env:
/// - VERIFIER (address): Verifier proxy address.
/// - LIQUIDITY_MANAGER (address): LiquidityManager proxy address.
/// - PRIVATE_KEY (uint256): Broadcaster private key.
/// Optional env:
/// - RELAY_OWNER (address): Owner for surplus withdrawals (defaults to broadcaster).
contract DeployGelatoRelay is DeterministicDeployer {
    error VerifierRequired();
    error LiquidityManagerRequired();

    function run() external {
        address verifier_ = vm.envAddress("VERIFIER");
        address liquidityManager_ = vm.envAddress("LIQUIDITY_MANAGER");
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address broadcaster = vm.addr(deployerKey);
        address relayOwner = vm.envOr("RELAY_OWNER", broadcaster);

        if (verifier_ == address(0)) revert VerifierRequired();
        if (liquidityManager_ == address(0)) revert LiquidityManagerRequired();

        vm.startBroadcast(deployerKey);
        bytes32 baseSalt = _loadBaseSalt();
        console2.log("Deploying GelatoRelay at block", block.number);

        // Deploy implementation
        bytes memory implCode =
            abi.encodePacked(type(GelatoRelay).creationCode, abi.encode(verifier_, liquidityManager_));
        address impl = _deploy3(broadcaster, baseSalt, "GELATO_RELAY_IMPL", implCode);

        // Deploy proxy with initializer
        bytes memory initCalldata = abi.encodeCall(GelatoRelay.initialize, (relayOwner, "GelatoRelay", "1"));
        address proxy = _deployProxyAndInit(broadcaster, baseSalt, "GELATO_RELAY", impl, initCalldata);

        console2.log("GelatoRelay impl at", impl);
        console2.log("GelatoRelay proxy at", proxy);
        console2.log("  verifier", verifier_);
        console2.log("  liquidityManager", liquidityManager_);
        console2.log("  owner", relayOwner);

        vm.stopBroadcast();
    }
}
