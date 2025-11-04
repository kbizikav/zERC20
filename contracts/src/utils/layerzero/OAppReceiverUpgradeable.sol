// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {OAppCoreUpgradeable} from "./OAppCoreUpgradeable.sol";
import {IOAppReceiver, Origin} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oapp/interfaces/IOAppReceiver.sol";

/**
 * @title OAppReceiverUpgradeable
 * @notice Upgradeable variant of LayerZero's OAppReceiver.
 */
abstract contract OAppReceiverUpgradeable is OAppCoreUpgradeable, IOAppReceiver {
    error OnlyEndpoint(address addr);

    uint64 internal constant RECEIVER_VERSION = 2;

    function oAppVersion() public view virtual override returns (uint64 senderVersion, uint64 receiverVersion) {
        return (0, RECEIVER_VERSION);
    }

    function isComposeMsgSender(Origin calldata, /*_origin*/ bytes calldata, /*_message*/ address _sender)
        public
        view
        virtual
        returns (bool)
    {
        return _sender == address(this);
    }

    function allowInitializePath(Origin calldata origin) public view virtual returns (bool) {
        return peers[origin.srcEid] == origin.sender;
    }

    function nextNonce(uint32, /*_srcEid*/ bytes32 /*_sender*/ ) public view virtual returns (uint64 nonce) {
        return 0;
    }

    function lzReceive(
        Origin calldata _origin,
        bytes32 _guid,
        bytes calldata _message,
        address _executor,
        bytes calldata _extraData
    ) public payable virtual {
        if (address(endpoint) != msg.sender) revert OnlyEndpoint(msg.sender);

        if (_getPeerOrRevert(_origin.srcEid) != _origin.sender) {
            revert OnlyPeer(_origin.srcEid, _origin.sender);
        }

        _lzReceive(_origin, _guid, _message, _executor, _extraData);
    }

    function _lzReceive(
        Origin calldata _origin,
        bytes32 _guid,
        bytes calldata _message,
        address _executor,
        bytes calldata _extraData
    ) internal virtual;

    uint256[48] private __gap;
}
