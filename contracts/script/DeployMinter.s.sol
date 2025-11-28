// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {console2} from "forge-std/console2.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";
import {Minter} from "../src/Minter.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";

/// @notice Deploys the Minter contract with deterministic salts.
/// Required env:
/// - MINTER_ZERC20_TOKEN (address) : Address of the zerc20 token exposing mint/burn.
/// - PRIVATE_KEY (uint256)        : Broadcaster private key.
/// Optional env:
/// - MINTER_TOKEN (address)      : Underlying token to wrap (defaults to native token when unset).
/// - MINTER_OWNER (address)      : Owner of the minter (defaults to broadcaster address).
contract DeployMinter is DeterministicDeployer {
    function run() external {
        address zerc20Token = vm.envAddress("MINTER_ZERC20_TOKEN");
        address token = vm.envOr("MINTER_TOKEN", address(0));
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address owner = vm.envOr("MINTER_OWNER", address(0));

        address broadcaster = vm.addr(deployerKey);
        if (owner == address(0)) {
            owner = broadcaster;
        }

        vm.startBroadcast(deployerKey);

        bytes32 baseSalt = _loadBaseSalt();
        console2.log("Deploying Minter at block", block.number);

        Minter implementation = new Minter{salt: _deriveSalt(baseSalt, "MINTER_IMPL")}();
        bytes memory initData = abi.encodeCall(Minter.initialize, (zerc20Token, token, owner));
        ERC1967Proxy proxy = new ERC1967Proxy{salt: _deriveSalt(baseSalt, "MINTER_PROXY")}(address(implementation), initData);
        Minter minter = Minter(address(proxy));

        console2.log("Minter implementation deployed at", address(implementation));
        console2.log("Minter proxy deployed at", address(minter));
        console2.log("  owner set to", owner);
        console2.log("  zerc20 token", zerc20Token);
        console2.log("  underlying token", token);

        vm.stopBroadcast();
    }
}
