// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script} from "forge-std/Script.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";

/// @notice Shared helpers for deterministic CREATE2 deployments across scripts.
abstract contract DeterministicDeployer is Script {
    bytes32 internal constant DEFAULT_DEPLOY_SALT = keccak256("zerc20.deploy.default");

    error ProxyInitFailed(bytes revertData);

    /// @dev Reads `DEPLOY_SALT` from the environment. Falls back to a fixed value when unset.
    function _loadBaseSalt() internal view returns (bytes32) {
        string memory saltEnv = vm.envOr("DEPLOY_SALT", string(""));
        if (bytes(saltEnv).length == 0) {
            return DEFAULT_DEPLOY_SALT;
        }
        return keccak256(bytes(saltEnv));
    }

    /// @dev Derives a contract-specific salt from the shared base salt.
    function _deriveSalt(bytes32 baseSalt, string memory label) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(baseSalt, label));
    }

    /// @dev Deploys an ERC1967 proxy with an empty `_data` payload so the CREATE2 init_code stays stable.
    function _deployProxy(bytes32 baseSalt, string memory label, address implementation)
        internal
        returns (address proxy)
    {
        proxy = address(new ERC1967Proxy{salt: _deriveSalt(baseSalt, label)}(implementation, ""));
    }

    /// @dev Deploys a proxy and then initializes it via delegatecall, reverting with the original error on failure.
    function _deployProxyAndInit(bytes32 baseSalt, string memory label, address implementation, bytes memory initCalldata)
        internal
        returns (address proxy)
    {
        proxy = _deployProxy(baseSalt, label, implementation);
        _initProxy(proxy, initCalldata);
    }

    /// @dev Runs the initializer on a freshly deployed proxy; bubbles up the revert reason when present.
    function _initProxy(address proxy, bytes memory initCalldata) internal {
        if (initCalldata.length == 0) return;
        (bool ok, bytes memory revertData) = proxy.call(initCalldata);
        if (!ok) {
            if (revertData.length != 0) {
                assembly {
                    revert(add(revertData, 0x20), mload(revertData))
                }
            }
            revert ProxyInitFailed(revertData);
        }
    }
}
