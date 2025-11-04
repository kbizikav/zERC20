// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";

import {Hub} from "../src/Hub.sol";
import {Verifier} from "../src/Verifier.sol";

/**
 * @notice Shared helpers for peer configuration scripts.
 */
abstract contract PeerScriptBase is Script {
    error EmptyArray(string name);
    error LengthMismatch(string lhs, string rhs);
    error EidOverflow(uint256 value);
    error ChainIdOverflow(uint256 value);

    function _addressToBytes32(address addr) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(addr)));
    }

    function _toUint32(uint256 value) internal pure returns (uint32) {
        if (value > type(uint32).max) {
            revert EidOverflow(value);
        }
        return uint32(value);
    }

    function _toUint64(uint256 value) internal pure returns (uint64) {
        if (value > type(uint64).max) {
            revert ChainIdOverflow(value);
        }
        return uint64(value);
    }

    function _requireNonEmpty(uint256 length, string memory name) internal pure {
        if (length == 0) {
            revert EmptyArray(name);
        }
    }

    function _requireEqualLength(uint256 lhs, uint256 rhs, string memory lhsName, string memory rhsName)
        internal
        pure
    {
        if (lhs != rhs) {
            revert LengthMismatch(lhsName, rhsName);
        }
    }
}

/**
 * @notice Broadcast from the hub chain to set peers for every deployed verifier.
 *
 * Environment:
 * - HUB_ADDRESS (address)           : Hub contract address on the current chain.
 * - VERIFIER_ADDRESSES (address[])  : Comma-separated list of verifier contract addresses.
 * - VERIFIER_EIDS (uint256[])       : Comma-separated list of remote verifier EIDs (one per address).
 * - TOKEN_ADDRESSES (address[])     : Comma-separated list of token addresses registered with each verifier.
 * - TOKEN_CHAIN_IDS (uint256[])     : Comma-separated list of remote chain IDs (EVM `chainid`) (one per verifier).
 *
 * Recommended order:
 * 1. Run this script on the hub chain to map each verifier's EID to its address.
 * 2. Run {SetVerifierPeers} on each verifier chain to link back to the hub.
 *
 * Example:
 * forge script script/SetPeers.s.sol:SetHubPeers --rpc-url $HUB_RPC --broadcast -vvvv
 */
contract SetHubPeers is PeerScriptBase {
    function run() external {
        address hubAddress = vm.envAddress("HUB_ADDRESS");
        address[] memory verifierAddresses = vm.envAddress("VERIFIER_ADDRESSES", ",");
        uint256[] memory verifierEidsRaw = vm.envUint("VERIFIER_EIDS", ",");
        address[] memory tokenAddresses = vm.envAddress("TOKEN_ADDRESSES", ",");
        uint256[] memory tokenChainIdsRaw = vm.envUint("TOKEN_CHAIN_IDS", ",");
        uint256 broadcasterKey = vm.envUint("PRIVATE_KEY");

        _requireNonEmpty(verifierAddresses.length, "VERIFIER_ADDRESSES");
        _requireEqualLength(verifierAddresses.length, verifierEidsRaw.length, "VERIFIER_ADDRESSES", "VERIFIER_EIDS");
        _requireEqualLength(verifierAddresses.length, tokenAddresses.length, "VERIFIER_ADDRESSES", "TOKEN_ADDRESSES");
        _requireEqualLength(verifierAddresses.length, tokenChainIdsRaw.length, "VERIFIER_ADDRESSES", "TOKEN_CHAIN_IDS");

        uint32[] memory verifierEids = new uint32[](verifierEidsRaw.length);
        for (uint256 i = 0; i < verifierEidsRaw.length; ++i) {
            verifierEids[i] = _toUint32(verifierEidsRaw[i]);
        }

        vm.startBroadcast(broadcasterKey);
        Hub hub = Hub(hubAddress);
        for (uint256 i = 0; i < verifierAddresses.length; ++i) {
            Hub.TokenInfo memory info = Hub.TokenInfo({
                chainId: _toUint64(tokenChainIdsRaw[i]),
                eid: verifierEids[i],
                verifier: verifierAddresses[i],
                token: tokenAddresses[i]
            });

            if (hub.eidToPosition(verifierEids[i]) == 0) {
                console2.log("Registering token for eid", uint256(verifierEids[i]));
                console2.log("  chainId", uint256(info.chainId));
                console2.log("  token address", tokenAddresses[i]);
                hub.registerToken(info);
            } else {
                console2.log("Updating token for eid", uint256(verifierEids[i]));
                console2.log("  chainId", uint256(info.chainId));
                console2.log("  token address", tokenAddresses[i]);
                hub.updateToken(info);
            }

            bytes32 peer = _addressToBytes32(verifierAddresses[i]);
            console2.log("Setting hub peer for eid", uint256(verifierEids[i]));
            console2.log("  peer address", verifierAddresses[i]);
            hub.setPeer(verifierEids[i], peer);
        }
        vm.stopBroadcast();
    }
}

/**
 * @notice Broadcast from a verifier chain to set the hub as its peer.
 *
 * Environment:
 * - HUB_ADDRESS (address)           : Hub contract address (remote).
 * - HUB_EID (uint256)               : Hub chain LayerZero endpoint ID.
 * - VERIFIER_ADDRESS (address)      : Verifier contract address on this chain.
 *
 * Example:
 * forge script script/SetPeers.s.sol:SetVerifierPeers --rpc-url $VERIFIER_RPC --broadcast -vvvv
 */
contract SetVerifierPeers is PeerScriptBase {
    function run() external {
        address verifierAddress = vm.envAddress("VERIFIER_ADDRESS");
        address hubAddress = vm.envAddress("HUB_ADDRESS");
        uint32 hubEid = _toUint32(vm.envUint("HUB_EID"));
        bytes32 hubPeer = _addressToBytes32(hubAddress);
        uint256 broadcasterKey = vm.envUint("PRIVATE_KEY");

        vm.startBroadcast(broadcasterKey);
        console2.log("Setting verifier peer", verifierAddress);
        console2.log("  hub address", hubAddress);
        console2.log("  hub eid", uint256(hubEid));
        Verifier(verifierAddress).setPeer(hubEid, hubPeer);
        vm.stopBroadcast();
    }
}
