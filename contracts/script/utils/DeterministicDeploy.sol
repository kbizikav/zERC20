// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Script} from "forge-std/Script.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";

/// @title Factory for deploying contracts to deterministic addresses via CREATE3
/// @author zefram.eth
/// @notice Interface for LI.FI's CREATE3Factory deployment
/// @dev See https://github.com/lifinance/create3-factory
interface ICREATE3Factory {
    /// @notice Deploys a contract using CREATE3
    /// @dev The provided salt is hashed together with msg.sender to generate the final salt
    /// @param salt The deployer-specific salt for determining the deployed contract's address
    /// @param creationCode The creation code of the contract to deploy
    /// @return deployed The address of the deployed contract
    function deploy(bytes32 salt, bytes memory creationCode) external payable returns (address deployed);

    /// @notice Predicts the address of a deployed contract
    /// @dev The provided salt is hashed together with the deployer address to generate the final salt
    /// @param deployer The deployer account that will call deploy()
    /// @param salt The deployer-specific salt for determining the deployed contract's address
    /// @return deployed The address of the contract that will be deployed
    function getDeployed(address deployer, bytes32 salt) external view returns (address deployed);
}

/// @notice Shared helpers for deterministic CREATE3 deployments across scripts.
/// @dev Uses external LI.FI CREATE3Factory to ensure atomic proxy deployment + initialization,
///      preventing potential MEV attacks on the intermediate CREATE3 proxy.
///      CREATE3 addresses depend only on deployer address and salt, not on init code,
///      enabling identical addresses across chains even when constructor arguments differ.
abstract contract DeterministicDeployer is Script {
    /// @dev LI.FI CREATE3Factory deployed at the same address on all supported chains.
    /// See https://github.com/lifinance/create3-factory for deployment list.
    ICREATE3Factory internal constant CREATE3_FACTORY = ICREATE3Factory(0x93FEC2C00BfE902F733B57c5a6CeeD7CD1384AE1);

    bytes32 internal constant DEFAULT_DEPLOY_SALT = keccak256("zerc20.deploy.default");

    error ProxyInitFailed(bytes revertData);

    /// @dev Ensures CREATE3Factory exists at the expected address.
    /// On local/test environments (Anvil) where LI.FI's factory isn't deployed, use
    /// `make setup-local` or run the following command before executing this script:
    ///   BYTECODE=$(cat out/CREATE3Factory.sol/CREATE3Factory.json | jq -r '.deployedBytecode.object')
    ///   cast rpc anvil_setCode 0x93FEC2C00BfE902F733B57c5a6CeeD7CD1384AE1 "$BYTECODE" --rpc-url <RPC_URL>
    function _ensureCreate3Factory() internal view {
        require(
            address(CREATE3_FACTORY).code.length > 0,
            "CREATE3Factory not deployed. Run 'make setup-local' first for local testing."
        );
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
    /// @param baseSalt Base salt shared across related deployments.
    /// @param label Human-readable label to derive unique salt.
    /// @param creationCode Bytecode including constructor arguments (abi.encodePacked(type(C).creationCode, abi.encode(args))).
    /// @return deployed The address of the deployed contract.
    function _deploy3(bytes32 baseSalt, string memory label, bytes memory creationCode)
        internal
        returns (address deployed)
    {
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
