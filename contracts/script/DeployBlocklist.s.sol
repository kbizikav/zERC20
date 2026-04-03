// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Script, console2} from "forge-std/Script.sol";
import {Blocklist} from "../src/Blocklist.sol";

/// @notice Deploys the shared Blocklist contract for OFAC sanctions compliance.
/// @dev Deploy once per chain. The resulting address is passed to zERC20 constructors.
/// Env:
/// - PRIVATE_KEY (uint256): Broadcaster / deployer key.
/// - BLOCKLIST_OWNER (address): Owner of the Blocklist (can add/remove entries). Defaults to deployer.
contract DeployBlocklist is Script {
    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);
        address owner = vm.envOr("BLOCKLIST_OWNER", deployer);

        console2.log("Deployer:", deployer);
        console2.log("Blocklist owner:", owner);

        vm.startBroadcast(deployerKey);

        Blocklist blocklist = new Blocklist(owner);

        vm.stopBroadcast();

        console2.log("Blocklist deployed at:", address(blocklist));
        console2.log("  owner:", blocklist.owner());
    }
}
