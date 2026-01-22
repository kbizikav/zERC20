// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";
import {LZAddressContext} from "lz-address-book/helpers/LZAddressContext.sol";
import {
    IMessageLibManager,
    SetConfigParam
} from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/IMessageLibManager.sol";
import {UlnConfig} from "@layerzerolabs/lz-evm-messagelib-v2/contracts/uln/UlnBase.sol";

/// @notice Sets LayerZero ULN config (confirmations + DVNs) for a single oapp + remote EID.
/// Required env:
/// - OAPP_ADDRESS (address): OApp address to configure on the current chain.
/// - REMOTE_EID (uint): Destination chain EID.
/// - CONFIRMATIONS (uint): ULN confirmation count (uint64).
/// - PRIVATE_KEY (uint256): Broadcaster private key.
/// Optional env:
/// - TARGET_LIB (string): "send", "receive", or "both" (default "both").
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
    error InvalidTargetLib(string value);

    uint32 internal constant CONFIG_TYPE_ULN = 2;

    struct EnvConfig {
        address oapp;
        uint32 remoteEid;
        uint64 confirmations;
        bool setSendLib;
        bool setReceiveLib;
        string[] requiredNames;
        string[] optionalNames;
        uint256 optionalThresholdRaw;
    }

    struct LzContext {
        LZAddressContext ctx;
        address endpoint;
        address sendLib;
        address receiveLib;
    }

    struct DvnConfig {
        address[] requiredDvns;
        address[] optionalDvns;
        uint8 requiredCount;
        uint8 optionalCount;
        uint8 optionalThreshold;
    }

    function run() external {
        EnvConfig memory env = _loadEnv();

        if (env.oapp == address(0)) revert EmptyOApp();

        LzContext memory lz = _initContext();
        DvnConfig memory dvn =
            _resolveAndValidateDvns(lz.ctx, env.requiredNames, env.optionalNames, env.optionalThresholdRaw);

        _applyConfig(env, lz, dvn);
    }

    function _loadEnv() private view returns (EnvConfig memory env) {
        env.oapp = vm.envAddress("OAPP_ADDRESS");
        env.remoteEid = _toUint32(vm.envUint("REMOTE_EID"));
        env.confirmations = _toUint64(vm.envUint("CONFIRMATIONS"));
        (env.setSendLib, env.setReceiveLib) = _parseTargetLib(vm.envOr("TARGET_LIB", string("both")));
        env.requiredNames = vm.envOr("REQUIRED_DVN_NAMES", ",", new string[](0));
        env.optionalNames = vm.envOr("OPTIONAL_DVN_NAMES", ",", new string[](0));
        env.optionalThresholdRaw = vm.envOr("OPTIONAL_DVN_THRESHOLD", uint256(0));
    }

    function _initContext() private returns (LzContext memory lz) {
        lz.ctx = new LZAddressContext();
        lz.ctx.setChainByChainId(block.chainid);
        lz.endpoint = lz.ctx.getEndpointV2();
        lz.sendLib = lz.ctx.getSendUln302();
        lz.receiveLib = lz.ctx.getReceiveUln302();
    }

    function _resolveAndValidateDvns(
        LZAddressContext ctx,
        string[] memory requiredNames,
        string[] memory optionalNames,
        uint256 optionalThresholdRaw
    ) private view returns (DvnConfig memory dvn) {
        (dvn.requiredDvns, dvn.optionalDvns) = _resolveAndSortDvns(ctx, requiredNames, optionalNames);
        (dvn.requiredCount, dvn.optionalCount, dvn.optionalThreshold) =
            _validateDvns(dvn.requiredDvns.length, dvn.optionalDvns.length, optionalThresholdRaw);
    }

    function _applyConfig(EnvConfig memory env, LzContext memory lz, DvnConfig memory dvn) private {
        if (!env.setSendLib && !env.setReceiveLib) {
            revert InvalidTargetLib("none");
        }
        UlnConfig memory config = _buildUlnConfig(
            env.confirmations,
            dvn.requiredCount,
            dvn.optionalCount,
            dvn.optionalThreshold,
            dvn.requiredDvns,
            dvn.optionalDvns
        );

        (bool updateSendLib, bool updateReceiveLib) =
            _getConfigUpdates(env, lz.endpoint, lz.sendLib, lz.receiveLib, config);

        if (!updateSendLib && !updateReceiveLib) {
            console2.log("ULN config already up to date; skipping broadcast");
            return;
        }

        _setConfig(
            env.oapp, env.remoteEid, lz.endpoint, lz.sendLib, lz.receiveLib, config, updateSendLib, updateReceiveLib
        );
        _logConfig(
            env.oapp,
            env.remoteEid,
            env.confirmations,
            lz.sendLib,
            lz.receiveLib,
            updateSendLib,
            updateReceiveLib,
            dvn.requiredDvns,
            dvn.optionalDvns,
            dvn.optionalThreshold
        );
    }

    function _resolveAndSortDvns(LZAddressContext ctx, string[] memory requiredNames, string[] memory optionalNames)
        private
        view
        returns (address[] memory requiredDvns, address[] memory optionalDvns)
    {
        requiredDvns = _resolveDvns(ctx, requiredNames);
        optionalDvns = _resolveDvns(ctx, optionalNames);
        _sortAddresses(requiredDvns);
        _sortAddresses(optionalDvns);
    }

    function _validateDvns(uint256 requiredLen, uint256 optionalLen, uint256 optionalThresholdRaw)
        private
        pure
        returns (uint8 requiredCount, uint8 optionalCount, uint8 optionalThreshold)
    {
        requiredCount = _toUint8(requiredLen);
        optionalCount = _toUint8(optionalLen);
        optionalThreshold = _toUint8(optionalThresholdRaw);

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
    }

    function _buildUlnConfig(
        uint64 confirmations,
        uint8 requiredCount,
        uint8 optionalCount,
        uint8 optionalThreshold,
        address[] memory requiredDvns,
        address[] memory optionalDvns
    ) private pure returns (UlnConfig memory) {
        return UlnConfig({
            confirmations: confirmations,
            requiredDVNCount: requiredCount,
            optionalDVNCount: optionalCount,
            optionalDVNThreshold: optionalThreshold,
            requiredDVNs: requiredDvns,
            optionalDVNs: optionalDvns
        });
    }

    function _getConfigUpdates(
        EnvConfig memory env,
        address endpoint,
        address sendLib,
        address receiveLib,
        UlnConfig memory config
    ) private view returns (bool updateSendLib, bool updateReceiveLib) {
        if (env.setSendLib) {
            updateSendLib = _needsConfigUpdate(env.oapp, endpoint, sendLib, env.remoteEid, config);
            if (!updateSendLib) {
                console2.log("Send lib config already up to date; skipping");
            }
        }
        if (env.setReceiveLib) {
            updateReceiveLib = _needsConfigUpdate(env.oapp, endpoint, receiveLib, env.remoteEid, config);
            if (!updateReceiveLib) {
                console2.log("Receive lib config already up to date; skipping");
            }
        }
    }

    function _needsConfigUpdate(address oapp, address endpoint, address lib, uint32 remoteEid, UlnConfig memory desired)
        private
        view
        returns (bool)
    {
        bytes memory current;
        // getConfig can revert when the lib is not registered or the EID has no default ULN config.
        // Treat that as "needs update" so we can apply the desired config instead of failing.
        try IMessageLibManager(endpoint).getConfig(oapp, lib, remoteEid, CONFIG_TYPE_ULN) returns (bytes memory value) {
            current = value;
        } catch {
            return true;
        }
        if (current.length == 0) {
            return true;
        }
        return keccak256(current) != keccak256(abi.encode(desired));
    }

    function _setConfig(
        address oapp,
        uint32 remoteEid,
        address endpoint,
        address sendLib,
        address receiveLib,
        UlnConfig memory config,
        bool setSendLib,
        bool setReceiveLib
    ) private {
        SetConfigParam[] memory params = new SetConfigParam[](1);
        params[0] = SetConfigParam({eid: remoteEid, configType: CONFIG_TYPE_ULN, config: abi.encode(config)});

        uint256 broadcasterKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(broadcasterKey);
        if (setSendLib) {
            IMessageLibManager(endpoint).setConfig(oapp, sendLib, params);
        }
        if (setReceiveLib) {
            IMessageLibManager(endpoint).setConfig(oapp, receiveLib, params);
        }
        vm.stopBroadcast();
    }

    function _logConfig(
        address oapp,
        uint32 remoteEid,
        uint64 confirmations,
        address sendLib,
        address receiveLib,
        bool setSendLib,
        bool setReceiveLib,
        address[] memory requiredDvns,
        address[] memory optionalDvns,
        uint8 optionalThreshold
    ) private pure {
        console2.log("Set ULN config for oapp", oapp);
        console2.log("  remote eid", uint256(remoteEid));
        console2.log("  confirmations", uint256(confirmations));
        if (setSendLib) {
            console2.log("  send lib", sendLib);
        }
        if (setReceiveLib) {
            console2.log("  receive lib", receiveLib);
        }
        _logDvns("required dvns", requiredDvns);
        _logDvns("optional dvns", optionalDvns);
        if (optionalDvns.length > 0) {
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

    function _logDvns(string memory label, address[] memory dvns) private pure {
        console2.log(" ", label, dvns.length);
        for (uint256 i = 0; i < dvns.length; ++i) {
            console2.log("   -", dvns[i]);
        }
    }

    function _parseTargetLib(string memory value) private pure returns (bool setSend, bool setReceive) {
        bytes32 hashed = keccak256(bytes(value));
        if (hashed == keccak256(bytes("send"))) {
            return (true, false);
        }
        if (hashed == keccak256(bytes("receive"))) {
            return (false, true);
        }
        if (hashed == keccak256(bytes("both"))) {
            return (true, true);
        }
        revert InvalidTargetLib(value);
    }

    function _toUint8(uint256 value) private pure returns (uint8) {
        if (value > type(uint8).max) {
            revert Uint8Overflow(value);
        }
        // casting to uint8 is safe because we check the upper bound above
        // forge-lint: disable-next-line(unsafe-typecast)
        return uint8(value);
    }

    function _toUint32(uint256 value) private pure returns (uint32) {
        if (value > type(uint32).max) {
            revert Uint32Overflow(value);
        }
        // casting to uint32 is safe because we check the upper bound above
        // forge-lint: disable-next-line(unsafe-typecast)
        return uint32(value);
    }

    function _toUint64(uint256 value) private pure returns (uint64) {
        if (value > type(uint64).max) {
            revert Uint64Overflow(value);
        }
        // casting to uint64 is safe because we check the upper bound above
        // forge-lint: disable-next-line(unsafe-typecast)
        return uint64(value);
    }
}
