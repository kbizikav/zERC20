// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Test} from "forge-std/Test.sol";
import {SelfCall} from "../../src/utils/SelfCall.sol";

/// @notice Simple test harness that exposes SelfCall functionality
contract SelfCallHarness is SelfCall {
    uint256 public counter;
    bool public onlySelfCallExecuted;

    /// @notice Function with enableSelfCall that calls an onlySelfCall function
    function executeWithSelfCall() external enableSelfCall {
        ++counter;
        this.protectedFunction();
    }

    /// @notice Function protected by onlySelfCall
    function protectedFunction() external onlySelfCall {
        onlySelfCallExecuted = true;
    }

    /// @notice Another enableSelfCall function to test nesting
    function nestedSelfCall() external enableSelfCall {
        counter += 10;
        // Intentionally try to enable again to test SelfCallAlreadyEnabled
        this.attemptDoubleEnable();
    }

    /// @notice Function that tries to enable self-call when already enabled
    function attemptDoubleEnable() external enableSelfCall {
        counter += 100;
    }

    /// @notice Reset state for testing
    function reset() external {
        counter = 0;
        onlySelfCallExecuted = false;
    }
}

contract SelfCallTest is Test {
    SelfCallHarness internal harness;

    function setUp() public {
        harness = new SelfCallHarness();
    }

    // ==================== Basic Functionality Tests ====================

    function testEnableSelfCallAllowsProtectedFunctionCall() public {
        assertEq(harness.counter(), 0, "counter should start at 0");
        assertFalse(harness.onlySelfCallExecuted(), "onlySelfCallExecuted should be false");

        harness.executeWithSelfCall();

        assertEq(harness.counter(), 1, "counter should increment");
        assertTrue(harness.onlySelfCallExecuted(), "onlySelfCallExecuted should be true");
    }

    function testProtectedFunctionRevertsWhenCalledExternally() public {
        vm.expectRevert(SelfCall.OnlySelfCall.selector);
        harness.protectedFunction();
    }

    function testProtectedFunctionRevertsWhenCalledByAnotherContract() public {
        address attacker = address(0x1337);

        vm.prank(attacker);
        vm.expectRevert(SelfCall.OnlySelfCall.selector);
        harness.protectedFunction();
    }

    // ==================== SelfCallAlreadyEnabled Tests ====================

    function testNestedEnableSelfCallReverts() public {
        vm.expectRevert(SelfCall.SelfCallAlreadyEnabled.selector);
        harness.nestedSelfCall();
    }

    // ==================== SelfCallNotAllowed Tests ====================

    function testSelfCallFromContractItselfButWithoutEnableReverts() public {
        // Even if msg.sender is address(this), without enableSelfCall it should fail
        vm.prank(address(harness));
        vm.expectRevert(SelfCall.SelfCallNotAllowed.selector);
        harness.protectedFunction();
    }

    // ==================== State Isolation Tests ====================

    function testEnableSelfCallCleansUpStateAfterExecution() public {
        // First call succeeds
        harness.executeWithSelfCall();
        assertEq(harness.counter(), 1);

        // Reset the flag
        harness.reset();

        // Second call should also succeed (state was cleaned up)
        harness.executeWithSelfCall();
        assertEq(harness.counter(), 1);
        assertTrue(harness.onlySelfCallExecuted());
    }

    function testMultipleSequentialSelfCallsSucceed() public {
        harness.executeWithSelfCall();
        assertEq(harness.counter(), 1);

        harness.executeWithSelfCall();
        assertEq(harness.counter(), 2);

        harness.executeWithSelfCall();
        assertEq(harness.counter(), 3);
    }

    // ==================== Edge Cases ====================

    function testProtectedFunctionRevertsWithZeroAddress() public {
        vm.prank(address(0));
        vm.expectRevert(SelfCall.OnlySelfCall.selector);
        harness.protectedFunction();
    }
}
