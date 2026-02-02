// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Script} from "forge-std/Script.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {ICREATE3Factory} from "create3-factory/ICREATE3Factory.sol";
import {
    FACTORY_ADDRESS,
    FACTORY_SALT,
    FACTORY_INIT_CODE,
    FACTORY_DEPLOYED_BYTECODE
} from "create3-factory/CREATE3Constant.sol";

/// @notice Shared helpers for deterministic CREATE3 deployments across scripts.
/// @dev Uses external CREATE3Factory to ensure atomic proxy deployment + initialization,
///      preventing potential MEV attacks on the intermediate CREATE3 proxy.
///      CREATE3 addresses depend only on deployer address and salt, not on init code,
///      enabling identical addresses across chains even when constructor arguments differ.
///      See https://github.com/InternetMaximalism/create3-factory
abstract contract DeterministicDeployer is Script {
    /// @dev CREATE3Factory deployed at the same address on all supported chains.
    /// See https://github.com/InternetMaximalism/create3-factory for deployment list.
    ICREATE3Factory internal constant CREATE3_FACTORY = ICREATE3Factory(FACTORY_ADDRESS);

    bytes32 internal constant DEFAULT_DEPLOY_SALT = keccak256("zerc20.deploy.default");

    /// @dev Deterministic Deployment Proxy (deployed on most EVM chains).
    /// See https://github.com/Arachnid/deterministic-deployment-proxy
    address internal constant DETERMINISTIC_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;

    error ProxyInitFailed(bytes revertData);
    error Create3FactoryDeploymentFailed();

    /// @dev Ensures CREATE3Factory exists at the expected address.
    /// On local/test environments (Anvil) where the factory isn't deployed,
    /// this function uses vm.etch to deploy the factory bytecode automatically.
    function _ensureCreate3Factory() internal {
        if (address(CREATE3_FACTORY).code.length == 0) {
            vm.etch(address(CREATE3_FACTORY), FACTORY_DEPLOYED_BYTECODE);
        }
    }

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

    /// @dev Deploys a contract using CREATE3 via external factory.
    /// The address depends only on the deployer (msg.sender to the factory) and salt.
    /// If CREATE3Factory is not deployed, it will be deployed first via Deterministic Deployment Proxy.
    /// @param baseSalt Base salt shared across related deployments.
    /// @param label Human-readable label to derive unique salt.
    /// @param creationCode Bytecode including constructor arguments (abi.encodePacked(type(C).creationCode, abi.encode(args))).
    /// @return deployed The address of the deployed contract.
    function _deploy3(bytes32 baseSalt, string memory label, bytes memory creationCode)
        internal
        returns (address deployed)
    {
        _deployCreate3FactoryIfNeeded();
        bytes32 salt = _deriveSalt(baseSalt, label);
        deployed = CREATE3_FACTORY.deploy(salt, creationCode);
    }

    /// @dev Deploys CREATE3Factory via Deterministic Deployment Proxy if not already deployed.
    /// The factory will be deployed at the same address on any chain.
    function _deployCreate3FactoryIfNeeded() internal {
        if (address(CREATE3_FACTORY).code.length > 0) {
            return;
        }

        // Deploy CREATE3Factory via Deterministic Deployment Proxy
        // The proxy expects calldata = salt ++ init_code
        bytes memory payload = abi.encodePacked(FACTORY_SALT, FACTORY_INIT_CODE);
        (bool success,) = DETERMINISTIC_DEPLOYER.call(payload);

        if (!success || address(CREATE3_FACTORY).code.length == 0) {
            revert Create3FactoryDeploymentFailed();
        }
    }

    /// @dev Predicts the CREATE3 address for a given deployer and salt without deploying.
    /// @param deployer The address that will call deploy() on the factory.
    /// @param baseSalt Base salt shared across related deployments.
    /// @param label Human-readable label to derive unique salt.
    /// @return predicted The address where the contract would be deployed.
    function _predictAddress(address deployer, bytes32 baseSalt, string memory label)
        internal
        view
        returns (address predicted)
    {
        bytes32 salt = _deriveSalt(baseSalt, label);
        predicted = CREATE3_FACTORY.getDeployed(deployer, salt);
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
