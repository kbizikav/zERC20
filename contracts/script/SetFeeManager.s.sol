// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";
import {LiquidityManager} from "../src/liquidity/LiquidityManager.sol";

/// @notice Grants FEE_MANAGER_ROLE to a specified address on LiquidityManager.
/// Required env:
/// - PRIVATE_KEY (uint256): Broadcaster private key (must have DEFAULT_ADMIN_ROLE).
/// - LIQUIDITY_MANAGER (address): LiquidityManager proxy address.
/// - FEE_MANAGER (address): Address to grant FEE_MANAGER_ROLE.
contract SetFeeManager is Script {
    error LiquidityManagerRequired();
    error FeeManagerRequired();

    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address liquidityManagerAddr = vm.envAddress("LIQUIDITY_MANAGER");
        address feeManagerAddr = vm.envAddress("FEE_MANAGER");

        if (liquidityManagerAddr == address(0)) revert LiquidityManagerRequired();
        if (feeManagerAddr == address(0)) revert FeeManagerRequired();

        LiquidityManager manager = LiquidityManager(payable(liquidityManagerAddr));
        bytes32 feeManagerRole = manager.FEE_MANAGER_ROLE();

        bool alreadyHasRole = manager.hasRole(feeManagerRole, feeManagerAddr);

        if (alreadyHasRole) {
            console2.log("FEE_MANAGER_ROLE already granted to", feeManagerAddr);
            console2.log("Skipping grantRole");
            return;
        }

        vm.startBroadcast(deployerKey);
        manager.grantRole(feeManagerRole, feeManagerAddr);
        vm.stopBroadcast();

        console2.log("FEE_MANAGER_ROLE granted to", feeManagerAddr);
        console2.log("LiquidityManager:", liquidityManagerAddr);
    }
}
