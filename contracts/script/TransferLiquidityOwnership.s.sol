// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";

interface IAccessControl {
    function hasRole(bytes32 role, address account) external view returns (bool);
    function grantRole(bytes32 role, address account) external;
    function revokeRole(bytes32 role, address account) external;
    function DEFAULT_ADMIN_ROLE() external view returns (bytes32);
}

interface ILiquidityManagerRoles is IAccessControl {
    function FEE_MANAGER_ROLE() external view returns (bytes32);
}

interface IOwnable {
    function owner() external view returns (address);
    function transferOwnership(address newOwner) external;
}

/// @notice Transfers all roles of a LiquidityManager to a new owner.
/// Grants DEFAULT_ADMIN_ROLE and FEE_MANAGER_ROLE to NEW_OWNER, then revokes from current admin.
/// Skips if target already has the role.
/// Required env:
/// - PRIVATE_KEY (uint256): Broadcaster private key (must be current admin).
/// - CONTRACT_ADDRESS (address): LiquidityManager proxy address.
/// - NEW_OWNER (address): New admin address to grant roles to.
contract TransferLiquidityManagerOwnership is Script {
    error ContractAddressRequired();
    error NewOwnerRequired();

    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address contractAddr = vm.envAddress("CONTRACT_ADDRESS");
        address newOwner = vm.envAddress("NEW_OWNER");
        address currentAdmin = vm.addr(deployerKey);

        if (contractAddr == address(0)) revert ContractAddressRequired();
        if (newOwner == address(0)) revert NewOwnerRequired();

        ILiquidityManagerRoles manager = ILiquidityManagerRoles(contractAddr);
        bytes32 adminRole = manager.DEFAULT_ADMIN_ROLE();
        bytes32 feeManagerRole = manager.FEE_MANAGER_ROLE();

        console2.log("LiquidityManager:", contractAddr);
        console2.log("Current admin:", currentAdmin);
        console2.log("Target owner:", newOwner);

        bool hasAdminRole = manager.hasRole(adminRole, newOwner);
        bool hasFeeManagerRole = manager.hasRole(feeManagerRole, newOwner);

        console2.log("Target has DEFAULT_ADMIN_ROLE:", hasAdminRole);
        console2.log("Target has FEE_MANAGER_ROLE:", hasFeeManagerRole);

        if (hasAdminRole && hasFeeManagerRole) {
            console2.log("Target already has all roles. Checking if revoke is needed...");
            bool currentHasAdmin = manager.hasRole(adminRole, currentAdmin);
            bool currentHasFeeManager = manager.hasRole(feeManagerRole, currentAdmin);
            if (!currentHasAdmin && !currentHasFeeManager) {
                console2.log("Current admin roles already revoked. Skipping.");
                return;
            }
        }

        vm.startBroadcast(deployerKey);

        // Re-check roles inside broadcast for accurate state after potential restart
        hasAdminRole = manager.hasRole(adminRole, newOwner);
        hasFeeManagerRole = manager.hasRole(feeManagerRole, newOwner);

        // Grant roles to new owner first (safe to call even if already granted)
        if (!hasAdminRole) {
            console2.log("Granting DEFAULT_ADMIN_ROLE to new owner...");
            manager.grantRole(adminRole, newOwner);
        }

        if (!hasFeeManagerRole) {
            console2.log("Granting FEE_MANAGER_ROLE to new owner...");
            manager.grantRole(feeManagerRole, newOwner);
        }

        // Revoke roles from current admin (only if different from new owner)
        // Re-check inside broadcast for accurate state after potential restart
        if (currentAdmin != newOwner) {
            bool currentHasFeeManager = manager.hasRole(feeManagerRole, currentAdmin);
            bool currentHasAdmin = manager.hasRole(adminRole, currentAdmin);

            if (currentHasFeeManager) {
                console2.log("Revoking FEE_MANAGER_ROLE from current admin...");
                manager.revokeRole(feeManagerRole, currentAdmin);
            }

            if (currentHasAdmin) {
                console2.log("Revoking DEFAULT_ADMIN_ROLE from current admin...");
                manager.revokeRole(adminRole, currentAdmin);
            }
        }

        vm.stopBroadcast();

        console2.log("LiquidityManager ownership transferred successfully");
    }
}

/// @notice Transfers ownership of an Adaptor to NEW_OWNER.
/// Skips if owner is already NEW_OWNER.
/// Required env:
/// - PRIVATE_KEY (uint256): Broadcaster private key (must be current owner).
/// - CONTRACT_ADDRESS (address): Adaptor proxy address.
/// - NEW_OWNER (address): New owner address to transfer to.
contract TransferAdaptorOwnership is Script {
    error ContractAddressRequired();
    error NewOwnerRequired();

    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address contractAddr = vm.envAddress("CONTRACT_ADDRESS");
        address newOwner = vm.envAddress("NEW_OWNER");

        if (contractAddr == address(0)) revert ContractAddressRequired();
        if (newOwner == address(0)) revert NewOwnerRequired();

        IOwnable adaptor = IOwnable(contractAddr);
        address currentOwner = adaptor.owner();

        console2.log("Adaptor:", contractAddr);
        console2.log("Current owner:", currentOwner);
        console2.log("Target owner:", newOwner);

        if (currentOwner == newOwner) {
            console2.log("Owner already set to target. Skipping.");
            return;
        }

        vm.startBroadcast(deployerKey);
        adaptor.transferOwnership(newOwner);
        vm.stopBroadcast();

        console2.log("Adaptor ownership transferred successfully");
    }
}
