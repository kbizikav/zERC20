// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Script} from "forge-std/Script.sol";
import {CREATE3} from "solady-0.1.8/src/utils/CREATE3.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";

/// @notice Shared helpers for deterministic CREATE3 deployments across scripts.
/// @dev CREATE3 addresses depend only on deployer address and salt, not on init code,
///      enabling identical addresses across chains even when constructor arguments differ.
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

    /// @dev Deploys a contract using CREATE3. The address depends only on msg.sender and salt.
    /// @param baseSalt Base salt shared across related deployments.
    /// @param label Human-readable label to derive unique salt.
    /// @param creationCode Bytecode including constructor arguments (abi.encodePacked(type(C).creationCode, abi.encode(args))).
    /// @return deployed The address of the deployed contract.
    function _deploy3(bytes32 baseSalt, string memory label, bytes memory creationCode)
        internal
        returns (address deployed)
    {
        bytes32 salt = _deriveSalt(baseSalt, label);
        deployed = CREATE3.deployDeterministic(creationCode, salt);
    }

    /// @dev Predicts the CREATE3 address for a given salt without deploying.
    /// @param baseSalt Base salt shared across related deployments.
    /// @param label Human-readable label to derive unique salt.
    /// @return predicted The address where the contract would be deployed.
    function _predictAddress(bytes32 baseSalt, string memory label) internal view returns (address predicted) {
        bytes32 salt = _deriveSalt(baseSalt, label);
        predicted = CREATE3.predictDeterministicAddress(salt);
    }

    /// @dev Deploys a proxy using CREATE3 and runs the initializer in a single call.
    function _deployProxyAndInit(
        bytes32 baseSalt,
        string memory label,
        address implementation,
        bytes memory initCalldata
    ) internal returns (address proxy) {
        bytes memory proxyCreationCode = abi.encodePacked(
            type(ERC1967Proxy).creationCode, abi.encode(implementation, initCalldata)
        );
        proxy = _deploy3(baseSalt, label, proxyCreationCode);
    }
}
