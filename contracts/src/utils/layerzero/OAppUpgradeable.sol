// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {OAppSenderUpgradeable} from "./OAppSenderUpgradeable.sol";
import {OAppReceiverUpgradeable} from "./OAppReceiverUpgradeable.sol";

/**
 * @title OAppUpgradeable
 * @notice Combines sender and receiver upgradeable variants.
 */
abstract contract OAppUpgradeable is OAppSenderUpgradeable, OAppReceiverUpgradeable {
    /// forge-lint: disable-next-line(mixed-case-function)
    function __OApp_init(address _endpoint, address _delegate) internal onlyInitializing {
        __OAppCore_init(_endpoint, _delegate);
    }

    function oAppVersion()
        public
        view
        virtual
        override(OAppSenderUpgradeable, OAppReceiverUpgradeable)
        returns (uint64 senderVersion, uint64 receiverVersion)
    {
        return (SENDER_VERSION, RECEIVER_VERSION);
    }

    uint256[50] private __gap;
}
