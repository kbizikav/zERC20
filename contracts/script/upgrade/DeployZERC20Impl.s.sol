// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.33;

import {console2} from "forge-std/console2.sol";
import {zERC20} from "../../src/zERC20.sol";
import {IBlocklist} from "../../src/interfaces/IBlocklist.sol";
import {DeterministicDeployer} from "../utils/DeterministicDeploy.sol";

/// @notice Deploys a new zERC20 implementation with Blocklist integration via CREATE3.
/// @dev Reads endpoint and decimals from the existing proxy so the new impl matches the
///      proxy's configuration (required by zERC20._authorizeUpgrade's endpoint check).
///      The proxy upgrade (upgradeToAndCall) must be executed separately by the proxy
///      owner (typically a Safe multisig).
/// Env:
/// - PRIVATE_KEY (uint256): Broadcaster / deployer key.
/// - ZERC20_PROXY (address): Existing zERC20 proxy to read endpoint and decimals from.
/// - BLOCKLIST_ADDRESS (address): Deployed Blocklist contract address on this chain.
/// - DEPLOY_SALT (string): Base salt override (e.g. "zbnb.impl.mainnet.v2").
contract DeployZERC20Impl is DeterministicDeployer {
    error ProxyMissing();
    error BlocklistMissing();

    function run() external {
        _ensureCreate3Factory();

        address proxy = vm.envAddress("ZERC20_PROXY");
        if (proxy == address(0)) revert ProxyMissing();
        address blocklistAddr = vm.envAddress("BLOCKLIST_ADDRESS");
        if (blocklistAddr == address(0)) revert BlocklistMissing();

        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);
        bytes32 baseSalt = _loadBaseSalt();

        zERC20 existing = zERC20(proxy);
        address endpoint = address(existing.endpoint());
        uint8 decimals = existing.decimals();

        address predicted = _predictAddress(deployer, baseSalt, "TOKEN_IMPL");

        console2.log("Chain ID:", block.chainid);
        console2.log("Deployer:", deployer);
        console2.log("Proxy:", proxy);
        console2.log("  name:", existing.name());
        console2.log("  symbol:", existing.symbol());
        console2.log("  endpoint:", endpoint);
        console2.log("  decimals:", decimals);
        console2.log("Blocklist:", blocklistAddr);
        console2.log("Predicted impl address:", predicted);

        vm.startBroadcast(deployerKey);

        bytes memory creationCode =
            abi.encodePacked(type(zERC20).creationCode, abi.encode(endpoint, decimals, blocklistAddr));
        address deployed = _deploy3(deployer, baseSalt, "TOKEN_IMPL", creationCode);

        vm.stopBroadcast();

        console2.log("New zERC20 impl deployed at:", deployed);
    }
}
