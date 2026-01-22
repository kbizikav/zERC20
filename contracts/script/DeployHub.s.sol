// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {console2} from "forge-std/console2.sol";
import {Hub} from "../src/Hub.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";
import {LZAddressContext} from "lz-address-book/helpers/LZAddressContext.sol";

/// @notice Deploys the Hub contract to Base Sepolia (or any chain) using config supplied via environment variables.
/// Required env:
/// - HUB_EID (uint)            : LayerZero endpoint id for the local chain (for logging/reference only).
/// Optional env:
/// - HUB_DELEGATE (address)    : Account allowed to manage LayerZero config (defaults to broadcaster).
/// - LayerZero endpoint is resolved automatically from lz-address-book using `block.chainid`.
contract DeployHub is DeterministicDeployer {
    function run() external {
        uint32 hubEid = uint32(vm.envUint("HUB_EID"));
        address endpoint = _resolveLzEndpoint();
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

        bytes memory hubImplCode = abi.encodePacked(type(Hub).creationCode, abi.encode(endpoint));
        Hub hubImpl = Hub(_deploy3(baseSalt, "HUB_IMPL", hubImplCode));
        bytes memory hubInit = abi.encodeCall(Hub.initialize, (delegate));
        Hub hub = Hub(_deployProxyAndInit(baseSalt, "HUB_PROXY", address(hubImpl), hubInit));

        console2.log("Hub implementation deployed at", address(hubImpl));
        console2.log("Hub proxy deployed at", address(hub));
        console2.log("Hub owner set to", delegate);

        vm.stopBroadcast();
    }

    function _resolveLzEndpoint() private returns (address endpoint) {
        LZAddressContext ctx = new LZAddressContext();
        ctx.setChainByChainId(block.chainid);
        endpoint = ctx.getEndpointV2();
    }
}
