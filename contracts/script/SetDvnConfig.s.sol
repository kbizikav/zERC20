// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";
import {LZAddressContext} from "lz-address-book/helpers/LZAddressContext.sol";
import {IMessageLibManager, SetConfigParam} from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/IMessageLibManager.sol";
import {UlnConfig} from "@layerzerolabs/lz-evm-messagelib-v2/contracts/uln/UlnBase.sol";

/// @notice Sets LayerZero ULN config (confirmations + DVNs) for a single oapp + remote EID.
/// Required env:
/// - OAPP_ADDRESS (address): OApp address to configure on the current chain.
/// - REMOTE_EID (uint): Destination chain EID.
/// - CONFIRMATIONS (uint): ULN confirmation count (uint64).
/// - PRIVATE_KEY (uint256): Broadcaster private key.
/// Optional env:
/// - REQUIRED_DVN_NAMES (string[]): Comma-separated DVN names (resolved via lz-address-book).
/// - OPTIONAL_DVN_NAMES (string[]): Comma-separated DVN names (resolved via lz-address-book).
/// - OPTIONAL_DVN_THRESHOLD (uint): Threshold for OPTIONAL_DVN_NAMES (required when optional dvns are set).
contract SetDvnConfig is Script {
    error EmptyOApp();
    error Uint8Overflow(uint256 value);
    error Uint32Overflow(uint256 value);
    error Uint64Overflow(uint256 value);
    error OptionalDVNThresholdMissing();
    error OptionalDVNThresholdTooHigh(uint256 threshold, uint256 count);
    error DVNConfigEmpty();

    uint32 internal constant CONFIG_TYPE_ULN = 2;

    function run() external {
        address oapp = vm.envAddress("OAPP_ADDRESS");
        uint32 remoteEid = _toUint32(vm.envUint("REMOTE_EID"));
        uint64 confirmations = _toUint64(vm.envUint("CONFIRMATIONS"));
        string[] memory requiredNames = vm.envOr("REQUIRED_DVN_NAMES", ",", new string[](0));
        string[] memory optionalNames = vm.envOr("OPTIONAL_DVN_NAMES", ",", new string[](0));
        uint256 optionalThresholdRaw = vm.envOr("OPTIONAL_DVN_THRESHOLD", uint256(0));

        if (oapp == address(0)) revert EmptyOApp();

        LZAddressContext ctx = new LZAddressContext();
        ctx.setChainByChainId(block.chainid);
        address endpoint = ctx.getEndpointV2();
        address sendLib = ctx.getSendUln302();
        address receiveLib = ctx.getReceiveUln302();

        address[] memory requiredDvns = _resolveDvns(ctx, requiredNames);
        address[] memory optionalDvns = _resolveDvns(ctx, optionalNames);
        _sortAddresses(requiredDvns);
        _sortAddresses(optionalDvns);

        uint8 requiredCount = _toUint8(requiredDvns.length);
        uint8 optionalCount = _toUint8(optionalDvns.length);
        uint8 optionalThreshold = _toUint8(optionalThresholdRaw);

        if (requiredCount == 0 && optionalThreshold == 0) {
            revert DVNConfigEmpty();
        }
        if (optionalCount == 0 && optionalThreshold != 0) {
            revert OptionalDVNThresholdTooHigh(optionalThreshold, optionalCount);
        }
        if (optionalCount > 0 && optionalThreshold == 0) {
            revert OptionalDVNThresholdMissing();
        }
        if (optionalThreshold > optionalCount) {
            revert OptionalDVNThresholdTooHigh(optionalThreshold, optionalCount);
        }

        UlnConfig memory config = UlnConfig({
            confirmations: confirmations,
            requiredDVNCount: requiredCount,
            optionalDVNCount: optionalCount,
            optionalDVNThreshold: optionalThreshold,
            requiredDVNs: requiredDvns,
            optionalDVNs: optionalDvns
        });

        SetConfigParam[] memory params = new SetConfigParam[](1);
        params[0] = SetConfigParam({eid: remoteEid, configType: CONFIG_TYPE_ULN, config: abi.encode(config)});

        uint256 broadcasterKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(broadcasterKey);
        IMessageLibManager(endpoint).setConfig(oapp, sendLib, params);
        IMessageLibManager(endpoint).setConfig(oapp, receiveLib, params);
        vm.stopBroadcast();

        console2.log("Set ULN config for oapp", oapp);
        console2.log("  remote eid", uint256(remoteEid));
        console2.log("  confirmations", uint256(confirmations));
        console2.log("  send lib", sendLib);
        console2.log("  receive lib", receiveLib);
        _logDvns("required dvns", requiredDvns);
        _logDvns("optional dvns", optionalDvns);
        if (optionalCount > 0) {
            console2.log("  optional threshold", uint256(optionalThreshold));
        }
    }

    function _resolveDvns(LZAddressContext ctx, string[] memory names) private view returns (address[] memory dvns) {
        if (names.length == 0) {
            return new address[](0);
        }
        dvns = new address[](names.length);
        for (uint256 i = 0; i < names.length; ++i) {
            dvns[i] = ctx.getDVNByName(names[i]);
        }
    }

    function _sortAddresses(address[] memory values) private pure {
        if (values.length < 2) {
            return;
        }
        for (uint256 i = 0; i < values.length - 1; ++i) {
            for (uint256 j = 0; j < values.length - 1 - i; ++j) {
                if (values[j] > values[j + 1]) {
                    address tmp = values[j];
                    values[j] = values[j + 1];
                    values[j + 1] = tmp;
                }
            }
        }
    }

    function _logDvns(string memory label, address[] memory dvns) private {
        console2.log(" ", label, dvns.length);
        for (uint256 i = 0; i < dvns.length; ++i) {
            console2.log("   -", dvns[i]);
        }
    }

    function _toUint8(uint256 value) private pure returns (uint8) {
        if (value > type(uint8).max) {
            revert Uint8Overflow(value);
        }
        return uint8(value);
    }

    function _toUint32(uint256 value) private pure returns (uint32) {
        if (value > type(uint32).max) {
            revert Uint32Overflow(value);
        }
        return uint32(value);
    }

    function _toUint64(uint256 value) private pure returns (uint64) {
        if (value > type(uint64).max) {
            revert Uint64Overflow(value);
        }
        return uint64(value);
    }
}
