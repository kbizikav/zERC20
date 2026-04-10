// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.33;

import {console2} from "forge-std/console2.sol";
import {Blocklist} from "../src/Blocklist.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";

/// @notice Deploys the shared Blocklist contract for OFAC sanctions compliance via CREATE3.
/// @dev Deploy once per chain. Uses CREATE3 to get the same address on all chains.
/// Env:
/// - PRIVATE_KEY (uint256): Broadcaster / deployer key.
/// - BLOCKLIST_OWNER (address): Owner of the Blocklist (can add/remove entries). Defaults to deployer.
/// - DEPLOY_SALT (string, optional): Base salt override.
contract DeployBlocklist is DeterministicDeployer {
    function run() external {
        _ensureCreate3Factory();

        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);
        address owner = vm.envOr("BLOCKLIST_OWNER", deployer);
        bytes32 baseSalt = _loadBaseSalt();

        address predicted = _predictAddress(deployer, baseSalt, "blocklist");

        console2.log("Deployer:", deployer);
        console2.log("Blocklist owner:", owner);
        console2.log("Predicted address:", predicted);

        vm.startBroadcast(deployerKey);

        bytes memory creationCode = abi.encodePacked(type(Blocklist).creationCode, abi.encode(owner));
        address deployed = _deploy3(deployer, baseSalt, "blocklist", creationCode);

        vm.stopBroadcast();

        console2.log("Blocklist deployed at:", deployed);
        console2.log("  owner:", Blocklist(deployed).owner());
    }
}
