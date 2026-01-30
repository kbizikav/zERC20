// SPDX-License-Identifier: AGPL-3.0
pragma solidity 0.8.33;

import {CREATE3} from "solady-0.1.8/src/utils/CREATE3.sol";

/// @title Factory for deploying contracts to deterministic addresses via CREATE3
/// @author zefram.eth (original), adapted to use Solady's CREATE3
/// @notice Enables deploying contracts using CREATE3. Each deployer (msg.sender) has
/// its own namespace for deployed addresses.
/// @dev This is a Solady-based implementation compatible with LI.FI's CREATE3Factory interface.
/// Used for local testing to match production behavior.
contract CREATE3Factory {
    /// @notice Deploys a contract using CREATE3
    /// @dev The provided salt is hashed together with msg.sender to generate the final salt
    /// @param salt The deployer-specific salt for determining the deployed contract's address
    /// @param creationCode The creation code of the contract to deploy
    /// @return deployed The address of the deployed contract
    function deploy(bytes32 salt, bytes memory creationCode) external payable returns (address deployed) {
        // hash salt with the deployer address to give each deployer its own namespace
        salt = keccak256(abi.encodePacked(msg.sender, salt));
        return CREATE3.deployDeterministic(msg.value, creationCode, salt);
    }

    /// @notice Predicts the address of a deployed contract
    /// @dev The provided salt is hashed together with the deployer address to generate the final salt
    /// @param deployer The deployer account that will call deploy()
    /// @param salt The deployer-specific salt for determining the deployed contract's address
    /// @return deployed The address of the contract that will be deployed
    function getDeployed(address deployer, bytes32 salt) external view returns (address deployed) {
        // hash salt with the deployer address to give each deployer its own namespace
        salt = keccak256(abi.encodePacked(deployer, salt));
        return CREATE3.predictDeterministicAddress(salt);
    }
}
