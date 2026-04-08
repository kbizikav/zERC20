// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.33;

import {Script, console2} from "forge-std/Script.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {zERC20} from "../src/zERC20.sol";

/// @notice Deploys the zERC20 token (new ITX) with UUPS proxy pattern.
contract DeployNewITX is Script {
    function run() external {
        address endpoint = 0x6EDCE65403992e310A62460808c4b910D972f10f;
        uint8 decimals = 18;
        string memory name = "new-itx";
        string memory symbol = "NITX";
        address initialOwner = 0x18DE9A6028cFAa0B4B58cc72E257b12e5625B396;

        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);

        console2.log("Deployer:", deployer);
        console2.log("Endpoint:", endpoint);
        console2.log("Decimals:", decimals);
        console2.log("Name:", name);
        console2.log("Symbol:", symbol);
        console2.log("Initial Owner:", initialOwner);

        vm.startBroadcast(deployerKey);

        // Deploy implementation
        zERC20 implementation = new zERC20(endpoint, decimals);
        console2.log("Implementation deployed at:", address(implementation));

        // Deploy proxy with initialization
        bytes memory initData = abi.encodeCall(zERC20.initialize, (name, symbol, initialOwner));
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        console2.log("Proxy deployed at:", address(proxy));

        vm.stopBroadcast();

        // Verify deployment
        zERC20 token = zERC20(address(proxy));
        console2.log("=== Verification ===");
        console2.log("Token name:", token.name());
        console2.log("Token symbol:", token.symbol());
        console2.log("Token decimals:", token.decimals());
        console2.log("Token owner:", token.owner());
    }
}
