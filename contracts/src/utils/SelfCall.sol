// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

abstract contract SelfCall {
    error OnlySelfCall();
    error SelfCallAlreadyEnabled();
    error SelfCallNotAllowed();

    bool private _isSelfCallAllowed;

    modifier enableSelfCall() {
        if (_isSelfCallAllowed) revert SelfCallAlreadyEnabled();
        _isSelfCallAllowed = true;
        _;
        _isSelfCallAllowed = false;
    }

    modifier onlySelfCall() {
        if (msg.sender != address(this)) revert OnlySelfCall();
        if (!_isSelfCallAllowed) revert SelfCallNotAllowed();
        _;
    }
}
