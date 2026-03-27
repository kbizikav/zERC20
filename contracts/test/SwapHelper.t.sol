// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Test} from "forge-std/Test.sol";
import {SwapHelper} from "../src/relay/SwapHelper.sol";
import {ERC20Permit, ERC20} from "@openzeppelin/contracts/token/ERC20/extensions/ERC20Permit.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {Initializable} from "@openzeppelin/contracts/proxy/utils/Initializable.sol";

/// @dev Minimal ERC20Permit token for testing.
contract MockERC20Permit is ERC20Permit {
    constructor() ERC20("Mock", "MCK") ERC20Permit("Mock") {}

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }
}

contract SwapHelperTest is Test {
    SwapHelper internal helper;
    MockERC20Permit internal token;

    address internal deployer;
    address internal relayer;
    uint256 internal relayerKey;
    uint256 internal ownerKey;
    address internal owner;
    address internal recipient;
    address internal attacker;

    bytes32 internal constant PERMIT_TYPEHASH =
        keccak256("Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)");

    event RelayerUpdated(address indexed relayer, bool allowed);

    function setUp() public {
        deployer = address(this);

        // Deploy upgradeable SwapHelper
        SwapHelper impl = new SwapHelper();
        bytes memory initData = abi.encodeCall(SwapHelper.initialize, (deployer));
        ERC1967Proxy proxy = new ERC1967Proxy(address(impl), initData);
        helper = SwapHelper(payable(address(proxy)));

        token = new MockERC20Permit();

        relayerKey = 0xBE1A;
        relayer = vm.addr(relayerKey);
        ownerKey = 0xA11CE;
        owner = vm.addr(ownerKey);
        recipient = address(0xBEEF);
        attacker = address(0xBAD);

        // Allowlist the relayer
        helper.setRelayer(relayer, true);

        // Fund relayer with native tokens
        vm.deal(relayer, 100 ether);
        vm.deal(attacker, 100 ether);
    }

    // --- helpers ---

    function _signPermit(
        uint256 signerKey,
        address _owner,
        address spender,
        uint256 value,
        uint256 nonce,
        uint256 deadline
    ) internal view returns (uint8 v, bytes32 r, bytes32 s) {
        bytes32 structHash = keccak256(abi.encode(PERMIT_TYPEHASH, _owner, spender, value, nonce, deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", token.DOMAIN_SEPARATOR(), structHash));
        (v, r, s) = vm.sign(signerKey, digest);
    }

    // --- core swap tests ---

    function test_swap_success() public {
        uint256 tokenAmount = 1000e18;
        uint256 nativeAmount = 0.5 ether;
        uint256 deadline = block.timestamp + 1 days;

        token.mint(owner, tokenAmount);
        (uint8 v, bytes32 r, bytes32 s) = _signPermit(ownerKey, owner, address(helper), tokenAmount, 0, deadline);

        uint256 recipientBalBefore = recipient.balance;
        uint256 relayerTokenBefore = token.balanceOf(relayer);

        vm.prank(relayer);
        helper.swap{value: nativeAmount}(address(token), owner, recipient, tokenAmount, deadline, v, r, s);

        assertEq(token.balanceOf(relayer), relayerTokenBefore + tokenAmount, "relayer received tokens");
        assertEq(recipient.balance, recipientBalBefore + nativeAmount, "recipient received native");
        assertEq(token.balanceOf(owner), 0, "owner tokens transferred");
        assertEq(address(helper).balance, 0, "helper has no leftover balance");
    }

    function test_swap_zero_value() public {
        uint256 tokenAmount = 500e18;
        uint256 deadline = block.timestamp + 1 days;

        token.mint(owner, tokenAmount);
        (uint8 v, bytes32 r, bytes32 s) = _signPermit(ownerKey, owner, address(helper), tokenAmount, 0, deadline);

        vm.prank(relayer);
        helper.swap{value: 0}(address(token), owner, recipient, tokenAmount, deadline, v, r, s);

        assertEq(token.balanceOf(relayer), tokenAmount, "relayer received tokens");
    }

    function test_swap_with_existing_allowance() public {
        uint256 tokenAmount = 1000e18;
        uint256 nativeAmount = 0.5 ether;
        uint256 deadline = block.timestamp + 1 days;

        token.mint(owner, tokenAmount);

        // Owner pre-approves SwapHelper
        vm.prank(owner);
        token.approve(address(helper), tokenAmount);

        // Use a bad permit signature — caught by try/catch, approve already set
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(0xDEAD, keccak256("bad"));

        vm.prank(relayer);
        helper.swap{value: nativeAmount}(address(token), owner, recipient, tokenAmount, deadline, v, r, s);

        assertEq(token.balanceOf(relayer), tokenAmount, "relayer received tokens via existing allowance");
        assertEq(recipient.balance, nativeAmount, "recipient received native");
    }

    function test_swap_reverts_no_allowance_bad_permit() public {
        uint256 tokenAmount = 1000e18;
        uint256 deadline = block.timestamp + 1 days;

        token.mint(owner, tokenAmount);

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(0xDEAD, keccak256("bad"));

        vm.prank(relayer);
        vm.expectRevert();
        helper.swap{value: 0.5 ether}(address(token), owner, recipient, tokenAmount, deadline, v, r, s);
    }

    function test_swap_reverts_recipient_rejects_native() public {
        uint256 tokenAmount = 1000e18;
        uint256 nativeAmount = 0.5 ether;
        uint256 deadline = block.timestamp + 1 days;

        token.mint(owner, tokenAmount);
        (uint8 v, bytes32 r, bytes32 s) = _signPermit(ownerKey, owner, address(helper), tokenAmount, 0, deadline);

        RejectEther rejecter = new RejectEther();

        vm.prank(relayer);
        vm.expectRevert(SwapHelper.NativeTransferFailed.selector);
        helper.swap{value: nativeAmount}(address(token), owner, address(rejecter), tokenAmount, deadline, v, r, s);

        // Atomicity: token transfer also reverted
        assertEq(token.balanceOf(owner), tokenAmount, "owner still has tokens");
        assertEq(token.balanceOf(relayer), 0, "relayer has no tokens");
    }

    function test_swap_atomicity_on_revert() public {
        uint256 tokenAmount = 1000e18;
        uint256 nativeAmount = 0.5 ether;
        uint256 deadline = block.timestamp + 1 days;

        // Do NOT mint tokens — transferFrom will fail
        (uint8 v, bytes32 r, bytes32 s) = _signPermit(ownerKey, owner, address(helper), tokenAmount, 0, deadline);

        uint256 relayerBalBefore = relayer.balance;

        vm.prank(relayer);
        vm.expectRevert();
        helper.swap{value: nativeAmount}(address(token), owner, recipient, tokenAmount, deadline, v, r, s);

        assertEq(relayer.balance, relayerBalBefore, "relayer ETH preserved on revert");
    }

    // --- allowlist tests ---

    function test_swap_reverts_if_not_allowlisted() public {
        uint256 tokenAmount = 1000e18;
        uint256 deadline = block.timestamp + 1 days;

        token.mint(owner, tokenAmount);
        (uint8 v, bytes32 r, bytes32 s) = _signPermit(ownerKey, owner, address(helper), tokenAmount, 0, deadline);

        vm.prank(attacker);
        vm.expectRevert(SwapHelper.NotAllowlisted.selector);
        helper.swap{value: 0.5 ether}(address(token), owner, recipient, tokenAmount, deadline, v, r, s);

        // Tokens not stolen
        assertEq(token.balanceOf(owner), tokenAmount, "owner keeps tokens");
        assertEq(token.balanceOf(attacker), 0, "attacker got nothing");
    }

    function test_setRelayer_emits_event() public {
        address newRelayer = address(0x1234);

        vm.expectEmit(true, false, false, true, address(helper));
        emit RelayerUpdated(newRelayer, true);
        helper.setRelayer(newRelayer, true);

        assertTrue(helper.isRelayer(newRelayer), "relayer added");
    }

    function test_setRelayer_remove() public {
        assertTrue(helper.isRelayer(relayer), "relayer initially allowed");

        helper.setRelayer(relayer, false);

        assertFalse(helper.isRelayer(relayer), "relayer removed");
    }

    function test_setRelayer_only_owner() public {
        vm.prank(attacker);
        vm.expectRevert(abi.encodeWithSelector(OwnableUpgradeable.OwnableUnauthorizedAccount.selector, attacker));
        helper.setRelayer(attacker, true);
    }

    // --- upgrade tests ---

    function test_upgrade_succeeds_as_owner() public {
        SwapHelper newImpl = new SwapHelper();
        helper.upgradeToAndCall(address(newImpl), bytes(""));

        // Still functional after upgrade
        assertTrue(helper.isRelayer(relayer), "state preserved");
    }

    function test_upgrade_reverts_non_owner() public {
        SwapHelper newImpl = new SwapHelper();

        vm.prank(attacker);
        vm.expectRevert(abi.encodeWithSelector(OwnableUpgradeable.OwnableUnauthorizedAccount.selector, attacker));
        helper.upgradeToAndCall(address(newImpl), bytes(""));
    }

    function test_impl_cannot_be_initialized() public {
        SwapHelper impl = new SwapHelper();
        vm.expectRevert(Initializable.InvalidInitialization.selector);
        impl.initialize(address(this));
    }
}

/// @dev Helper contract that cannot receive ETH.
contract RejectEther {
    receive() external payable {
        revert("no ETH");
    }
}
