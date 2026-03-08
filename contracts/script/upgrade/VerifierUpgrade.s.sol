// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";
import {Verifier} from "../../src/Verifier.sol";

/// @notice Deploys a fresh Verifier implementation and upgrades the existing proxy with `initializeV2(...)`.
/// @dev Relay fee authorizations require the Verifier EIP-712 domain to be initialized in the same transaction.
/// Env:
/// - VERIFIER_PROXY (address): Verifier proxy address to upgrade.
/// - PRIVATE_KEY (uint256): Broadcaster key that owns the proxy.
/// Optional env:
/// - EIP712_NAME (string): EIP-712 domain name. Defaults to "Verifier".
/// - EIP712_VERSION (string): EIP-712 domain version. Defaults to "1".
contract UpgradeVerifier is Script {
    // From EIP-1967: bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1)
    bytes32 internal constant IMPLEMENTATION_SLOT = 0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc;

    error VerifierProxyMissing();

    function run() external {
        address proxy = vm.envAddress("VERIFIER_PROXY");
        if (proxy == address(0)) revert VerifierProxyMissing();

        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        string memory eip712Name = vm.envOr("EIP712_NAME", string("Verifier"));
        string memory eip712Version = vm.envOr("EIP712_VERSION", string("1"));

        vm.startBroadcast(deployerKey);

        address previousImpl = _getImplementation(proxy);
        address endpoint = address(Verifier(proxy).endpoint());
        Verifier newImpl = new Verifier(endpoint);
        bytes memory initData = abi.encodeCall(Verifier.initializeV2, (eip712Name, eip712Version));

        console2.log("Upgrading Verifier");
        console2.log("  proxy", proxy);
        console2.log("  previous impl", previousImpl);
        console2.log("  new impl", address(newImpl));
        console2.log("  endpoint", endpoint);
        console2.log("  eip712 name", eip712Name);
        console2.log("  eip712 version", eip712Version);

        Verifier(proxy).upgradeToAndCall(address(newImpl), initData);

        console2.log("Upgrade complete");
        console2.log("  active impl", _getImplementation(proxy));

        vm.stopBroadcast();
    }

    function _getImplementation(address proxy) private view returns (address impl) {
        impl = address(uint160(uint256(vm.load(proxy, IMPLEMENTATION_SLOT))));
    }
}
