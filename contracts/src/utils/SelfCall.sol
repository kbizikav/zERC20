// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {TransientSlot} from "@openzeppelin/contracts/utils/TransientSlot.sol";

/// @title SelfCall
/// @notice Abstract contract providing a secure self-call pattern using transient storage.
/// @dev Uses EIP-1153 transient storage to track self-call authorization within a transaction.
///      The `enableSelfCall` modifier sets a transient flag that allows functions protected by
///      `onlySelfCall` to be invoked via `address(this).call(...)`. The flag is automatically
///      cleared at transaction end, ensuring no persistent state pollution.
abstract contract SelfCall {
    using TransientSlot for bytes32;
    using TransientSlot for TransientSlot.BooleanSlot;

    /// @notice Thrown when a function is called by an address other than the contract itself.
    error OnlySelfCall();

    /// @notice Thrown when attempting to enable self-call while it is already enabled.
    error SelfCallAlreadyEnabled();

    /// @notice Thrown when a self-call protected function is called without the flag being set.
    error SelfCallNotAllowed();

    /// @dev ERC-7201 namespaced storage slot for "zerc20.storage.SelfCall".
    ///      Computed as: keccak256(abi.encode(uint256(keccak256("zerc20.storage.SelfCall")) - 1)) & ~bytes32(uint256(0xff))
    bytes32 internal constant SELF_CALL_STORAGE = 0xb9bf29a13c3c2e77b212ed63d4dd1d38fe904bdd58adce08407bd5715a4eaf00;

    /// @notice Modifier that enables self-call for the duration of the function execution.
    /// @dev Sets the transient flag before execution and clears it after.
    ///      Reverts with `SelfCallAlreadyEnabled` if nested.
    modifier enableSelfCall() {
        _enableSelfCallBefore();
        _;
        _enableSelfCallAfter();
    }

    /// @notice Modifier that restricts function access to self-calls only.
    /// @dev Requires both `msg.sender == address(this)` and the transient flag to be set.
    modifier onlySelfCall() {
        _onlySelfCall();
        _;
    }

    /// @dev Enables the self-call flag. Reverts if already enabled to prevent nesting.
    function _enableSelfCallBefore() internal {
        require(!SELF_CALL_STORAGE.asBoolean().tload(), SelfCallAlreadyEnabled());
        SELF_CALL_STORAGE.asBoolean().tstore(true);
    }

    /// @dev Disables the self-call flag after function execution.
    function _enableSelfCallAfter() internal {
        SELF_CALL_STORAGE.asBoolean().tstore(false);
    }

    /// @dev Validates that the caller is the contract itself and the self-call flag is set.
    ///      Reverts with `OnlySelfCall` if msg.sender is not this contract.
    ///      Reverts with `SelfCallNotAllowed` if the transient flag is not set.
    function _onlySelfCall() internal view {
        require(msg.sender == address(this), OnlySelfCall());
        require(SELF_CALL_STORAGE.asBoolean().tload(), SelfCallNotAllowed());
    }
}
