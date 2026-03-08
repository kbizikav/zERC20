// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";
import {GelatoRelay} from "../../src/relay/GelatoRelay.sol";

/// @notice Deploys a fresh GelatoRelay implementation and upgrades an existing proxy to it.
/// Env:
/// - GELATO_RELAY_PROXY (address): Proxy address to upgrade.
/// - PRIVATE_KEY (uint256): Broadcaster key that owns the proxy.
contract UpgradeGelatoRelay is Script {
    // From EIP-1967: bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1)
    bytes32 internal constant IMPLEMENTATION_SLOT = 0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc;

    error GelatoRelayProxyMissing();

    function run() external {
        address proxy = vm.envAddress("GELATO_RELAY_PROXY");
        if (proxy == address(0)) revert GelatoRelayProxyMissing();

        uint256 deployerKey = vm.envUint("PRIVATE_KEY");

        vm.startBroadcast(deployerKey);

        address previousImpl = _getImplementation(proxy);
        GelatoRelay current = GelatoRelay(payable(proxy));
        address verifier = address(current.VERIFIER());
        address liquidityManager = address(current.LIQUIDITY_MANAGER());
        GelatoRelay newImpl = new GelatoRelay(verifier, liquidityManager);

        console2.log("Upgrading GelatoRelay");
        console2.log("  proxy", proxy);
        console2.log("  previous impl", previousImpl);
        console2.log("  new impl", address(newImpl));
        console2.log("  verifier", verifier);
        console2.log("  liquidityManager", liquidityManager);

        current.upgradeToAndCall(address(newImpl), bytes(""));

        console2.log("Upgrade complete");
        console2.log("  active impl", _getImplementation(proxy));

        vm.stopBroadcast();
    }

    function _getImplementation(address proxy) private view returns (address impl) {
        impl = address(uint160(uint256(vm.load(proxy, IMPLEMENTATION_SLOT))));
    }
}
