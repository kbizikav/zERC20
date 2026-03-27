// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {console2} from "forge-std/console2.sol";
import {SwapHelper} from "../src/relay/SwapHelper.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";

/// @notice Deploys the SwapHelper contract (UUPS proxy) using CREATE3.
/// After deployment, call `setRelayer()` to allowlist relayer addresses.
/// Required env:
/// - PRIVATE_KEY (uint)            : Deployer private key.
/// Optional env:
/// - SWAP_HELPER_OWNER (address)   : Owner of the SwapHelper (defaults to deployer).
/// - RELAYER (address)             : Relayer address to allowlist.
contract DeploySwapHelper is DeterministicDeployer {
    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);
        address owner = vm.envOr("SWAP_HELPER_OWNER", deployer);

        vm.startBroadcast(deployerKey);
        console2.log("Deploying SwapHelper from", deployer);

        // Implementation
        address impl = _deploy3Global(deployer, "SwapHelper_IMPL", type(SwapHelper).creationCode);
        console2.log("SwapHelper implementation at", impl);

        // Proxy
        bytes memory initData = abi.encodeCall(SwapHelper.initialize, (owner));
        bytes memory proxyCreationCode = abi.encodePacked(type(ERC1967Proxy).creationCode, abi.encode(impl, initData));
        address proxy = _deploy3Global(deployer, "SwapHelper_PROXY", proxyCreationCode);
        console2.log("SwapHelper proxy at", proxy);
        console2.log("SwapHelper owner", owner);

        address relayer = vm.envOr("RELAYER", address(0));
        if (relayer != address(0)) {
            SwapHelper(proxy).setRelayer(relayer, true);
            console2.log("SwapHelper relayer set", relayer);
        }

        vm.stopBroadcast();
    }
}
