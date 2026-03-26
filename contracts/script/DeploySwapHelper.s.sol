// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {console2} from "forge-std/console2.sol";
import {SwapHelper} from "../src/SwapHelper.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";

/// @notice Deploys the SwapHelper contract (UUPS proxy) using CREATE3.
/// After deployment, call `setRelayer()` to allowlist relayer addresses.
/// Required env:
/// - PRIVATE_KEY (uint)            : Deployer private key.
/// Optional env:
/// - SWAP_HELPER_OWNER (address)   : Owner of the SwapHelper (defaults to deployer).
/// - DEPLOY_SALT (string)          : Custom salt (defaults to "zerc20.deploy.default").
contract DeploySwapHelper is DeterministicDeployer {
    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);
        bytes32 baseSalt = _loadBaseSalt();
        address owner = vm.envOr("SWAP_HELPER_OWNER", deployer);

        vm.startBroadcast(deployerKey);
        console2.log("Deploying SwapHelper from", deployer);

        // Implementation
        address impl = _deploy3(deployer, baseSalt, "SwapHelper_IMPL", type(SwapHelper).creationCode);
        console2.log("SwapHelper implementation at", impl);

        // Proxy
        bytes memory initData = abi.encodeCall(SwapHelper.initialize, (owner));
        address proxy = _deployProxyAndInit(deployer, baseSalt, "SwapHelper_PROXY", impl, initData);
        console2.log("SwapHelper proxy at", proxy);
        console2.log("SwapHelper owner", owner);

        vm.stopBroadcast();
    }
}
