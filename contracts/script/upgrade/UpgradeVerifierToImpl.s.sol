// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";
import {Verifier} from "../../src/Verifier.sol";

/// @notice Upgrades an existing Verifier proxy to a pre-deployed implementation and calls `initializeV2(...)`.
/// @dev Unlike VerifierUpgrade.s.sol this script does NOT deploy a new implementation —
///      it reuses one that was already deployed (e.g. via DeployVerifierImpl.s.sol).
/// Env:
/// - VERIFIER_PROXY (address): Verifier proxy address to upgrade.
/// - NEW_IMPL (address): Already-deployed Verifier implementation address.
/// - PRIVATE_KEY (uint256): Broadcaster key that owns the proxy.
/// Optional env:
/// - EIP712_NAME (string): EIP-712 domain name. Defaults to "Verifier".
/// - EIP712_VERSION (string): EIP-712 domain version. Defaults to "1".
contract UpgradeVerifierToImpl is Script {
    // From EIP-1967: bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1)
    bytes32 internal constant IMPLEMENTATION_SLOT = 0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc;

    error VerifierProxyMissing();
    error NewImplMissing();

    function run() external {
        address proxy = vm.envAddress("VERIFIER_PROXY");
        if (proxy == address(0)) revert VerifierProxyMissing();

        address newImpl = vm.envAddress("NEW_IMPL");
        if (newImpl == address(0)) revert NewImplMissing();

        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        string memory eip712Name = vm.envOr("EIP712_NAME", string("Verifier"));
        string memory eip712Version = vm.envOr("EIP712_VERSION", string("1"));

        vm.startBroadcast(deployerKey);

        address previousImpl = _getImplementation(proxy);
        bytes memory initData = abi.encodeCall(Verifier.initializeV2, (eip712Name, eip712Version));

        console2.log("Upgrading Verifier to pre-deployed impl");
        console2.log("  proxy", proxy);
        console2.log("  previous impl", previousImpl);
        console2.log("  new impl", newImpl);
        console2.log("  eip712 name", eip712Name);
        console2.log("  eip712 version", eip712Version);

        Verifier(proxy).upgradeToAndCall(newImpl, initData);

        console2.log("Upgrade complete");
        console2.log("  active impl", _getImplementation(proxy));

        vm.stopBroadcast();
    }

    function _getImplementation(address proxy) private view returns (address impl) {
        impl = address(uint160(uint256(vm.load(proxy, IMPLEMENTATION_SLOT))));
    }
}
