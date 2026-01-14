// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

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

    /// @dev Deploys a proxy and runs the initializer in a single call.
    ///      The proxy init data remains empty to keep the CREATE2 init code stable.
    function _deployProxyAndInit(
        bytes32 baseSalt,
        string memory label,
        address implementation,
        bytes memory initCalldata
    ) internal returns (address proxy) {
        proxy = address(new ERC1967Proxy{salt: _deriveSalt(baseSalt, label)}(implementation, initCalldata));
    }
}
