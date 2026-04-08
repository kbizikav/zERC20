// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.33;

import {console2} from "forge-std/console2.sol";
import {Verifier} from "../../src/Verifier.sol";
import {DeterministicDeployer} from "../utils/DeterministicDeploy.sol";
import {LZAddressContext} from "lz-address-book/helpers/LZAddressContext.sol";

/// @notice Deploys a fresh Verifier implementation via CREATE3 for UUPS upgrade.
/// The proxy upgrade must still be executed with `upgradeToAndCall(...)` so `initializeV2(...)`
/// runs in the same transaction for relay EIP-712 support.
/// Env:
/// - PRIVATE_KEY (uint256): Deployer key.
contract DeployVerifierImpl is DeterministicDeployer {
    bytes32 internal constant SALT = keccak256("zerc20.verifier.impl.v3");

    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);

        LZAddressContext ctx = new LZAddressContext();
        ctx.setChainByChainId(block.chainid);
        address endpoint = ctx.getEndpointV2();

        address predicted = _predictAddress(deployer, SALT, "VERIFIER_IMPL");
        console2.log("Chain ID:", block.chainid);
        console2.log("Endpoint:", endpoint);
        console2.log("Predicted address:", predicted);

        vm.startBroadcast(deployerKey);

        bytes memory creationCode = abi.encodePacked(type(Verifier).creationCode, abi.encode(endpoint));
        address impl = _deploy3(deployer, SALT, "VERIFIER_IMPL", creationCode);

        console2.log("New Verifier impl:", impl);

        vm.stopBroadcast();
    }
}
