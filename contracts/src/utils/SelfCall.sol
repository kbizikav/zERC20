// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

abstract contract SelfCall {
    // comment errorはinterfaceに切り分けたほうがいいかなと。
    error OnlySelfCall();
    error SelfCallAlreadyEnabled();
    error SelfCallNotAllowed();
    // comment ReentrancyGuardTransientのような機構を使うとガス代節約になります。
    bool private _isSelfCallAllowed;

    modifier enableSelfCall() {
        if (_isSelfCallAllowed) revert SelfCallAlreadyEnabled();
        // ストレージに書き込むとガス代がかかるので、
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
