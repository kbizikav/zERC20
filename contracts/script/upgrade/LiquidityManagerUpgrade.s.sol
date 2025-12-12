// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";
import {LiquidityManager} from "../../src/liquidity/LiquidityManager.sol";

/// @notice Deploys a fresh LiquidityManager implementation and upgrades the existing proxy to point to it.
/// Env:
/// - LIQUIDITY_MANAGER (address): Proxy address to upgrade.
/// - PRIVATE_KEY (uint256): Broadcaster key that holds DEFAULT_ADMIN_ROLE on the proxy.
contract UpgradeLiquidityManager is Script {
    // From EIP-1967: bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1)
    bytes32 internal constant IMPLEMENTATION_SLOT = 0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc;

    error LiquidityManagerProxyMissing();

    function run() external {
        address proxy = vm.envAddress("LIQUIDITY_MANAGER");
        if (proxy == address(0)) revert LiquidityManagerProxyMissing();

        uint256 deployerKey = vm.envUint("PRIVATE_KEY");

        vm.startBroadcast(deployerKey);

        address previousImpl = _getImplementation(proxy);
        LiquidityManager newImpl = new LiquidityManager();

        console2.log("Upgrading LiquidityManager");
        console2.log("  proxy", proxy);
        console2.log("  previous impl", previousImpl);
        console2.log("  new impl", address(newImpl));

        LiquidityManager(proxy).upgradeTo(address(newImpl));

        console2.log("Upgrade complete");
        console2.log("  active impl", _getImplementation(proxy));

        vm.stopBroadcast();
    }

    function _getImplementation(address proxy) private view returns (address impl) {
        impl = address(uint160(uint256(vm.load(proxy, IMPLEMENTATION_SLOT))));
    }
}
