// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.33;

import {Script} from "forge-std/Script.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {ICREATE3Factory} from "create3-factory/ICREATE3Factory.sol";
import {FACTORY_ADDRESS} from "create3-factory/CREATE3Constant.sol";

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

    /// @dev Global salt for stateless verifier contracts (RootDecider, WithdrawDeciders, Groth16Verifiers).
    /// These contracts have no constructor arguments and can be shared across all token deployments
    /// (e.g., zUSDC, zETH) on the same chain, saving deployment gas.
    bytes32 internal constant GLOBAL_VERIFIERS_SALT = keccak256("zerc20.verifiers.global");

    error Create3FactoryNotDeployed();

    /// @dev Ensures CREATE3Factory exists at the expected address.
    /// Reverts if the factory is not deployed on the current chain.
    function _ensureCreate3Factory() internal view {
        if (address(CREATE3_FACTORY).code.length == 0) {
            revert Create3FactoryNotDeployed();
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
    /// If a contract already exists at the predicted address, deployment is skipped.
    /// @param deployer The address that will call deploy() on the factory (typically vm.addr(privateKey)).
    /// @param baseSalt Base salt shared across related deployments.
    /// @param label Human-readable label to derive unique salt.
    /// @param creationCode Bytecode including constructor arguments (abi.encodePacked(type(C).creationCode, abi.encode(args))).
    /// @return deployed The address of the deployed contract.
    function _deploy3(address deployer, bytes32 baseSalt, string memory label, bytes memory creationCode)
        internal
        returns (address deployed)
    {
        deployed = _predictAddress(deployer, baseSalt, label);

        // Skip deployment if contract already exists
        if (deployed.code.length > 0) {
            return deployed;
        }
        bytes32 salt = _deriveSalt(baseSalt, label);
        deployed = CREATE3_FACTORY.deploy(salt, creationCode);
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
    /// @param deployer The address that will call deploy() on the factory (typically vm.addr(privateKey)).
    function _deployProxyAndInit(
        address deployer,
        bytes32 baseSalt,
        string memory label,
        address implementation,
        bytes memory initCalldata
    ) internal returns (address proxy) {
        bytes memory proxyCreationCode = abi.encodePacked(
            type(ERC1967Proxy).creationCode, abi.encode(implementation, initCalldata)
        );
        proxy = _deploy3(deployer, baseSalt, label, proxyCreationCode);
    }

    /// @dev Deploys a contract using the global verifiers salt namespace.
    /// Use this for stateless verifier contracts that can be shared across token deployments.
    /// @param deployer The address that will call deploy() on the factory.
    /// @param label Human-readable label to derive unique salt within the global namespace.
    /// @param creationCode Bytecode including constructor arguments.
    /// @return deployed The address of the deployed contract.
    function _deploy3Global(address deployer, string memory label, bytes memory creationCode)
        internal
        returns (address deployed)
    {
        deployed = _deploy3(deployer, GLOBAL_VERIFIERS_SALT, label, creationCode);
    }

    /// @dev Predicts the CREATE3 address for a global verifier contract.
    /// @param deployer The address that will call deploy() on the factory.
    /// @param label Human-readable label to derive unique salt within the global namespace.
    /// @return predicted The address where the contract would be deployed.
    function _predictGlobalAddress(address deployer, string memory label) internal view returns (address predicted) {
        predicted = _predictAddress(deployer, GLOBAL_VERIFIERS_SALT, label);
    }
}
