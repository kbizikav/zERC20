// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {console2} from "forge-std/console2.sol";
import {Hub} from "../src/Hub.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";

/// @notice Deploys the Hub contract to Base Sepolia (or any chain) using config supplied via environment variables.
/// Required env:
/// - HUB_EID (uint)            : LayerZero endpoint id for the local chain (for logging/reference only).
/// - HUB_ENDPOINT (address)    : LayerZero endpoint address on the local chain.
/// Optional env:
/// - HUB_DELEGATE (address)    : Account allowed to manage LayerZero config (defaults to broadcaster).
contract DeployHub is DeterministicDeployer {
    function run() external {
        uint32 hubEid = uint32(vm.envUint("HUB_EID"));
        address endpoint = vm.envAddress("HUB_ENDPOINT");
        address delegate = vm.envOr("HUB_DELEGATE", address(0));
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);
        bytes32 baseSalt = _loadBaseSalt();

        vm.startBroadcast(deployerKey);
        console2.log("Deploying Hub for eid", hubEid);
        console2.log("Broadcasting from", deployer);
        if (delegate == address(0)) {
            delegate = deployer;
        }

        Hub hubImpl = new Hub{salt: _deriveSalt(baseSalt, "HUB_IMPL")}(endpoint);
        bytes memory hubInit = abi.encodeCall(Hub.initialize, (delegate));
        ERC1967Proxy proxy = new ERC1967Proxy{salt: _deriveSalt(baseSalt, "HUB_PROXY")}(address(hubImpl), hubInit);
        Hub hub = Hub(address(proxy));

        console2.log("Hub implementation deployed at", address(hubImpl));
        console2.log("Hub proxy deployed at", address(hub));
        console2.log("Hub owner set to", delegate);

        vm.stopBroadcast();
    }
}
