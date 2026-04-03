// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";
import {zERC20} from "../../src/zERC20.sol";
import {IBlocklist} from "../../src/interfaces/IBlocklist.sol";

/// @notice Deploys a new zERC20 implementation with Blocklist immutable and upgrades an existing proxy.
/// @dev The Blocklist contract must be deployed first via DeployBlocklist.s.sol.
/// Env:
/// - ZERC20_PROXY (address): zERC20 proxy address to upgrade.
/// - BLOCKLIST_ADDRESS (address): Deployed Blocklist contract address on this chain.
/// - PRIVATE_KEY (uint256): Broadcaster key that owns the proxy.
contract UpgradeZERC20Blocklist is Script {
    // From EIP-1967: bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1)
    bytes32 internal constant IMPLEMENTATION_SLOT = 0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc;

    error ProxyMissing();
    error BlocklistMissing();

    function run() external {
        address proxy = vm.envAddress("ZERC20_PROXY");
        if (proxy == address(0)) revert ProxyMissing();
        address blocklistAddr = vm.envAddress("BLOCKLIST_ADDRESS");
        if (blocklistAddr == address(0)) revert BlocklistMissing();

        uint256 deployerKey = vm.envUint("PRIVATE_KEY");

        vm.startBroadcast(deployerKey);

        address previousImpl = _getImplementation(proxy);
        zERC20 existing = zERC20(proxy);
        address endpoint = address(existing.endpoint());
        uint8 decimals = existing.decimals();

        zERC20 newImpl = new zERC20(endpoint, decimals, IBlocklist(blocklistAddr));

        console2.log("Upgrading zERC20 with Blocklist");
        console2.log("  proxy:", proxy);
        console2.log("  token:", existing.name(), existing.symbol());
        console2.log("  previous impl:", previousImpl);
        console2.log("  new impl:", address(newImpl));
        console2.log("  endpoint:", endpoint);
        console2.log("  decimals:", decimals);
        console2.log("  blocklist:", blocklistAddr);

        existing.upgradeToAndCall(address(newImpl), bytes(""));

        console2.log("Upgrade complete");
        console2.log("  active impl:", _getImplementation(proxy));
        console2.log("  BLOCKLIST:", address(zERC20(proxy).BLOCKLIST()));

        vm.stopBroadcast();
    }

    function _getImplementation(address proxy) private view returns (address impl) {
        impl = address(uint160(uint256(vm.load(proxy, IMPLEMENTATION_SLOT))));
    }
}
