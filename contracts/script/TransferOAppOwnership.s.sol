// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";

interface IOwnableOApp {
    function owner() external view returns (address);
    function transferOwnership(address newOwner) external;
    function setDelegate(address delegate) external;
    function endpoint() external view returns (address);
}

interface ILayerZeroEndpointV2 {
    function delegates(address oapp) external view returns (address);
}

/// @notice Sets the delegate of an OApp to NEW_OWNER via LayerZero Endpoint.
/// Skips if delegate is already set to NEW_OWNER.
/// Required env:
/// - PRIVATE_KEY (uint256): Broadcaster private key (must be current owner).
/// - OAPP_ADDRESS (address): OApp contract address.
/// - NEW_OWNER (address): New delegate address to set.
contract SetOAppDelegate is Script {
    error OAppAddressRequired();
    error NewOwnerRequired();

    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address oappAddr = vm.envAddress("OAPP_ADDRESS");
        address newOwner = vm.envAddress("NEW_OWNER");

        if (oappAddr == address(0)) revert OAppAddressRequired();
        if (newOwner == address(0)) revert NewOwnerRequired();

        IOwnableOApp oapp = IOwnableOApp(oappAddr);
        address endpointAddr = oapp.endpoint();
        ILayerZeroEndpointV2 endpoint = ILayerZeroEndpointV2(endpointAddr);

        address currentDelegate = endpoint.delegates(oappAddr);

        console2.log("OApp:", oappAddr);
        console2.log("Current delegate:", currentDelegate);
        console2.log("Target delegate:", newOwner);

        if (currentDelegate == newOwner) {
            console2.log("Delegate already set to target. Skipping.");
            return;
        }

        vm.startBroadcast(deployerKey);
        oapp.setDelegate(newOwner);
        vm.stopBroadcast();

        console2.log("Delegate updated successfully");
    }
}

/// @notice Transfers ownership of an OApp to NEW_OWNER.
/// Skips if owner is already NEW_OWNER.
/// Required env:
/// - PRIVATE_KEY (uint256): Broadcaster private key (must be current owner).
/// - OAPP_ADDRESS (address): OApp contract address.
/// - NEW_OWNER (address): New owner address to transfer to.
contract TransferOAppOwner is Script {
    error OAppAddressRequired();
    error NewOwnerRequired();

    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address oappAddr = vm.envAddress("OAPP_ADDRESS");
        address newOwner = vm.envAddress("NEW_OWNER");

        if (oappAddr == address(0)) revert OAppAddressRequired();
        if (newOwner == address(0)) revert NewOwnerRequired();

        IOwnableOApp oapp = IOwnableOApp(oappAddr);
        address currentOwner = oapp.owner();

        console2.log("OApp:", oappAddr);
        console2.log("Current owner:", currentOwner);
        console2.log("Target owner:", newOwner);

        if (currentOwner == newOwner) {
            console2.log("Owner already set to target. Skipping.");
            return;
        }

        vm.startBroadcast(deployerKey);
        oapp.transferOwnership(newOwner);
        vm.stopBroadcast();

        console2.log("Ownership transferred successfully");
    }
}
