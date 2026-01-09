// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {TransientSlot} from "@openzeppelin/contracts/utils/TransientSlot.sol";

abstract contract SelfCall {
    using TransientSlot for *;

    error OnlySelfCall();
    error SelfCallAlreadyEnabled();
    error SelfCallNotAllowed();

    // keccak256(abi.encode(uint256(keccak256("zerc20.storage.SelfCall")) - 1)) & ~bytes32(uint256(0xff))
    bytes32 private constant SELF_CALL_STORAGE =
        0xb9bf29a13c3c2e77b212ed63d4dd1d38fe904bdd58adce08407bd5715a4eaf00;

    modifier enableSelfCall() {
        if (SELF_CALL_STORAGE.asBoolean().tload()) revert SelfCallAlreadyEnabled();
        SELF_CALL_STORAGE.asBoolean().tstore(true);
        _;
        SELF_CALL_STORAGE.asBoolean().tstore(false);
    }

    modifier onlySelfCall() {
        if (msg.sender != address(this)) revert OnlySelfCall();
        if (!SELF_CALL_STORAGE.asBoolean().tload()) revert SelfCallNotAllowed();
        _;
    }
}
