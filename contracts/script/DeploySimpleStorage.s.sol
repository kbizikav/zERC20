// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Script, console} from "forge-std/Script.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";

/// @notice Simple storage contract for testing CREATE3 deployment and verification.
contract SimpleStorage {
    uint256 public value;
    address public owner;

    event ValueSet(uint256 newValue);

    constructor(uint256 _initialValue) {
        value = _initialValue;
        owner = msg.sender;
    }

    function setValue(uint256 _value) external {
        value = _value;
        emit ValueSet(_value);
    }
}

contract DeploySimpleStorage is DeterministicDeployer {
    function run() external {
        _ensureCreate3Factory();

        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);
        bytes32 baseSalt = _loadBaseSalt();
        uint256 initialValue = 42;

        console.log("Deploying SimpleStorage with CREATE3...");
        console.log("Base salt:", vm.toString(baseSalt));
        console.log("Deployer:", deployer);

        vm.startBroadcast(deployerKey);

        bytes memory creationCode = abi.encodePacked(type(SimpleStorage).creationCode, abi.encode(initialValue));

        address deployed = _deploy3(deployer, baseSalt, "SimpleStorage", creationCode);

        vm.stopBroadcast();

        console.log("SimpleStorage deployed at:", deployed);
        console.log("Initial value:", SimpleStorage(deployed).value());
    }
}
