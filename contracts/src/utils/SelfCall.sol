// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {TransientSlot} from "@openzeppelin/contracts/utils/TransientSlot.sol";

abstract contract SelfCall {
    using TransientSlot for *;

    error OnlySelfCall();
    error SelfCallAlreadyEnabled();
    error SelfCallNotAllowed();

    // ERC-7201 slot for namespace "zerc20.storage.SelfCall".
    bytes32 internal constant SELF_CALL_STORAGE = 0xb9bf29a13c3c2e77b212ed63d4dd1d38fe904bdd58adce08407bd5715a4eaf00;

    modifier enableSelfCall() {
        _enableSelfCallBefore();
        _;
        _enableSelfCallAfter();
    }

    modifier onlySelfCall() {
        _onlySelfCall();
        _;
    }

    function _enableSelfCallBefore() internal {
        if (SELF_CALL_STORAGE.asBoolean().tload()) revert SelfCallAlreadyEnabled();
        SELF_CALL_STORAGE.asBoolean().tstore(true);
    }

    function _enableSelfCallAfter() internal {
        SELF_CALL_STORAGE.asBoolean().tstore(false);
    }

    function _onlySelfCall() internal view {
        if (msg.sender != address(this)) revert OnlySelfCall();
        if (!SELF_CALL_STORAGE.asBoolean().tload()) revert SelfCallNotAllowed();
    }
}
