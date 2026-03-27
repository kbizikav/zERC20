// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Test} from "forge-std/Test.sol";
import {Blocklist} from "../src/Blocklist.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

contract BlocklistTest is Test {
    Blocklist internal bl;
    address internal constant ALICE = address(0xA11CE);
    address internal constant BOB = address(0xB0B);

    event AddressBlocked(address indexed account);
    event AddressUnblocked(address indexed account);

    function setUp() public {
        bl = new Blocklist(address(this));
    }

    function testConstructorRejectsZeroOwner() public {
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableInvalidOwner.selector, address(0)));
        new Blocklist(address(0));
    }

    function testBlockAndUnblock() public {
        assertFalse(bl.isBlocked(ALICE), "not blocked initially");

        vm.expectEmit(true, false, false, false, address(bl));
        emit AddressBlocked(ALICE);
        bl.blockAddress(ALICE);
        assertTrue(bl.isBlocked(ALICE), "blocked");

        vm.expectEmit(true, false, false, false, address(bl));
        emit AddressUnblocked(ALICE);
        bl.unblockAddress(ALICE);
        assertFalse(bl.isBlocked(ALICE), "unblocked");
    }

    function testBlockAddressRejectsZero() public {
        vm.expectRevert(Blocklist.ZeroAddress.selector);
        bl.blockAddress(address(0));
    }

    function testUnblockAddressRejectsZero() public {
        vm.expectRevert(Blocklist.ZeroAddress.selector);
        bl.unblockAddress(address(0));
    }

    function testBlockAddressesBatch() public {
        address[] memory accounts = new address[](3);
        accounts[0] = address(0x1);
        accounts[1] = address(0x2);
        accounts[2] = address(0x3);

        bl.blockAddresses(accounts);

        assertTrue(bl.isBlocked(address(0x1)), "0x1 blocked");
        assertTrue(bl.isBlocked(address(0x2)), "0x2 blocked");
        assertTrue(bl.isBlocked(address(0x3)), "0x3 blocked");
    }

    function testBlockAddressesBatchRejectsZero() public {
        address[] memory accounts = new address[](2);
        accounts[0] = address(0x1);
        accounts[1] = address(0);

        vm.expectRevert(Blocklist.ZeroAddress.selector);
        bl.blockAddresses(accounts);
    }

    function testOnlyOwnerCanBlock() public {
        vm.prank(ALICE);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, ALICE));
        bl.blockAddress(BOB);
    }

    function testOnlyOwnerCanUnblock() public {
        bl.blockAddress(BOB);
        vm.prank(ALICE);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, ALICE));
        bl.unblockAddress(BOB);
    }

    function testOnlyOwnerCanBlockBatch() public {
        address[] memory accounts = new address[](1);
        accounts[0] = BOB;
        vm.prank(ALICE);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, ALICE));
        bl.blockAddresses(accounts);
    }
}
