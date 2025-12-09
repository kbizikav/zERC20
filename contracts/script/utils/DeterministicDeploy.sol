// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script} from "forge-std/Script.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";

/// @notice Shared helpers for deterministic CREATE2 deployments across scripts.
contract DeterministicProxyFactory {
    error ProxyInitFailed(bytes revertData);

    /// @dev Deploys a proxy with deterministic CREATE2 bytecode and runs the initializer atomically.
    ///      The proxy create2 init code is identical to the previous flow (empty `_data`), so addresses stay stable
    ///      for a given factory address + salt.
    function deployAndInit(bytes32 salt, address implementation, bytes calldata initCalldata)
        external
        returns (address proxy)
    {
        proxy = address(new ERC1967Proxy{salt: salt}(implementation, ""));
        if (initCalldata.length == 0) return proxy;

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

abstract contract DeterministicDeployer is Script {
    bytes32 internal constant DEFAULT_DEPLOY_SALT = keccak256("zerc20.deploy.default");
    string internal constant FACTORY_LABEL = "PROXY_FACTORY";

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

    /// @dev Deploys a proxy and initializes it atomically through a deterministic factory.
    function _deployProxyAndInit(bytes32 baseSalt, string memory label, address implementation, bytes memory initCalldata)
        internal
        returns (address proxy)
    {
        DeterministicProxyFactory factory = _deployFactoryIfNeeded(baseSalt);
        proxy = factory.deployAndInit(_deriveSalt(baseSalt, label), implementation, initCalldata);
    }

    /// @dev Ensures the deterministic factory exists for the provided base salt and returns it.
    ///      The factory itself is deployed via CREATE2 so its address is stable per (deployer, baseSalt).
    function _deployFactoryIfNeeded(bytes32 baseSalt) private returns (DeterministicProxyFactory factory) {
        address deployer = _currentDeployer();
        bytes32 factorySalt = _deriveSalt(baseSalt, FACTORY_LABEL);
        bytes32 initCodeHash = keccak256(type(DeterministicProxyFactory).creationCode);
        address predicted = _computeCreate2Address(deployer, factorySalt, initCodeHash);

        if (predicted.code.length == 0) {
            factory = new DeterministicProxyFactory{salt: factorySalt}();
        } else {
            factory = DeterministicProxyFactory(predicted);
        }
    }

    function _computeCreate2Address(address deployer, bytes32 salt, bytes32 initCodeHash)
        private
        pure
        returns (address)
    {
        return address(uint160(uint256(keccak256(abi.encodePacked(bytes1(0xff), deployer, salt, initCodeHash)))));
    }

    /// @dev Returns the address used as the CREATE2 deployer when broadcasting. Falls back to env PRIVATE_KEY for dry-runs.
    function _currentDeployer() private returns (address) {
        // During broadcast this is the broadcaster EOA; in dry-runs fall back to PRIVATE_KEY or the script address.
        address origin = tx.origin;
        if (origin != address(0)) {
            return origin;
        }
        uint256 key = vm.envOr("PRIVATE_KEY", uint256(0));
        if (key != 0) {
            return vm.addr(key);
        }
        return address(this);
    }
}
