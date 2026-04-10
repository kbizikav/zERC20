// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.33;

import {Test} from "forge-std/Test.sol";
import {
    EndpointV2Mock as EndpointV2
} from "@layerzerolabs/test-devtools-evm-foundry/contracts/mocks/EndpointV2Mock.sol";
import {IOAppCore} from "@layerzerolabs/oapp-evm/contracts/oapp/interfaces/IOAppCore.sol";
import {zERC20} from "../src/zERC20.sol";
import {IBlocklist} from "../src/interfaces/IBlocklist.sol";
import {Blocklist} from "../src/Blocklist.sol";
import {ShaHashChainLib} from "../src/utils/ShaHashChainLib.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {IOFT, SendParam} from "@layerzerolabs/oft-evm/contracts/interfaces/IOFT.sol";
import {
    ERC20PermitUpgradeable
} from "@openzeppelin/contracts-upgradeable/token/ERC20/extensions/ERC20PermitUpgradeable.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {Initializable} from "@openzeppelin/contracts/proxy/utils/Initializable.sol";

contract ZERC20Harness is zERC20 {
    constructor(address endpoint, uint8 decimals_, IBlocklist blocklist_) zERC20(endpoint, decimals_, blocklist_) {}

    function debit(uint256 amountToSendLd, uint256 minAmountToCreditLd, uint32 dstEid)
        public
        returns (uint256 amountDebitedLd, uint256 amountToCreditLd)
    {
        return _debit(msg.sender, amountToSendLd, minAmountToCreditLd, dstEid);
    }

    function debitView(uint256 amountToSendLd, uint256 minAmountToCreditLd, uint32 dstEid)
        public
        view
        returns (uint256 amountDebitedLd, uint256 amountToCreditLd)
    {
        return _debitView(amountToSendLd, minAmountToCreditLd, dstEid);
    }

    function credit(address to, uint256 amountToCreditLd, uint32 srcEid) public returns (uint256 amountReceivedLd) {
        return _credit(to, amountToCreditLd, srcEid);
    }

    function removeDust(uint256 amountLd) public view returns (uint256) {
        return _removeDust(amountLd);
    }

    function toLd(uint64 amountSd) public view returns (uint256) {
        return _toLD(amountSd);
    }

    function toSd(uint256 amountLd) public view returns (uint64) {
        return _toSD(amountLd);
    }

    function buildMsgAndOptions(SendParam calldata sendParam, uint256 amountToCreditLd)
        public
        view
        returns (bytes memory message, bytes memory options)
    {
        return _buildMsgAndOptions(sendParam, amountToCreditLd);
    }
}

contract ZERC20Test is Test {
    ZERC20Harness internal token;
    Blocklist internal bl;
    EndpointV2 internal endpoint;

    address internal constant ALICE = address(0xA11CE);
    address internal constant BOB = address(0xB0B);

    bytes32 internal constant PERMIT_TYPEHASH =
        keccak256("Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)");

    event IndexedTransfer(uint256 indexed index, address from, address to, uint256 value);
    event Teleport(address indexed to, uint256 value);
    event VerifierUpdated(address indexed newVerifier);
    event MinterUpdated(address indexed newMinter);

    function setUp() public {
        endpoint = new EndpointV2(1, address(this));
        bl = new Blocklist(address(this));
        token = _deployToken(address(this), endpoint, 18);
        token.setMinter(address(this));
    }

    function _deployToken(address owner, EndpointV2 endpointMock, uint8 decimals_) private returns (ZERC20Harness) {
        ZERC20Harness impl = new ZERC20Harness(address(endpointMock), decimals_, IBlocklist(address(bl)));
        bytes memory initData = abi.encodeCall(zERC20.initialize, ("Zero Token", "ZTK", owner));
        ERC1967Proxy proxy = new ERC1967Proxy(address(impl), initData);
        return ZERC20Harness(address(proxy));
    }

    function testConstructorRevertsOnZeroEndpoint() public {
        vm.expectRevert(IOAppCore.InvalidEndpointCall.selector);
        new zERC20(address(0), 18, IBlocklist(address(bl)));
    }

    function testConstructorRevertsOnZeroBlocklist() public {
        vm.expectRevert(zERC20.ZeroAddress.selector);
        new zERC20(address(endpoint), 18, IBlocklist(address(0)));
    }

    function testInitializeRevertsOnZeroOwner() public {
        ZERC20Harness impl = new ZERC20Harness(address(endpoint), 18, IBlocklist(address(bl)));
        bytes memory initData = abi.encodeCall(zERC20.initialize, ("Test", "TST", address(0)));

        vm.expectRevert(zERC20.ZeroAddress.selector);
        new ERC1967Proxy(address(impl), initData);
    }

    function testInitializeCannotBeCalledTwice() public {
        vm.expectRevert(Initializable.InvalidInitialization.selector);
        token.initialize("Again", "AGAIN", address(this));
    }

    function testImplementationInitializeIsDisabled() public {
        ZERC20Harness impl = new ZERC20Harness(address(endpoint), 18, IBlocklist(address(bl)));
        vm.expectRevert(Initializable.InvalidInitialization.selector);
        impl.initialize("Impl", "IMPL", address(this));
    }

    function testHashChainMatchesZkpVector() public pure {
        address from = address(0x1111111111111111111111111111111111111111);
        address to = address(0x2222222222222222222222222222222222222222);
        uint256 value = 0x333;
        uint256 expected = 0x00b499aa085c64d5668ec9512d24a54cb7cf7174543dd1dd5a806f77d0bb3e93;

        uint256 actual = ShaHashChainLib.compute(0, from, to, value);
        assertEq(actual, expected, "hash chain should align with zk circuit vector");
    }

    function testMintInitializesHashChainAndIndex() public {
        uint256 amount = 5 ether;

        vm.expectEmit(true, true, false, true, address(token));
        emit IndexedTransfer(0, address(0), ALICE, amount);
        token.mint(ALICE, amount);

        assertEq(token.balanceOf(ALICE), amount, "minted balance");
        assertEq(token.index(), 1, "index after mint");

        uint256 expectedHash = ShaHashChainLib.compute(0, address(0), ALICE, amount);
        assertEq(token.hashChain(), expectedHash, "hash chain after mint");
    }

    function testTransferChainsHashAndEmitsIndexed() public {
        uint256 mintAmount = 10 ether;
        uint256 transferAmount = 3 ether;

        token.mint(ALICE, mintAmount);
        uint256 previousHash = token.hashChain();
        uint256 startIndex = token.index();

        vm.expectEmit(true, true, false, true, address(token));
        emit IndexedTransfer(startIndex, ALICE, BOB, transferAmount);
        vm.prank(ALICE);
        bool transferOk = token.transfer(BOB, transferAmount);
        assertTrue(transferOk, "transfer should succeed");

        assertEq(token.balanceOf(ALICE), mintAmount - transferAmount, "alice balance");
        assertEq(token.balanceOf(BOB), transferAmount, "bob balance");
        assertEq(token.index(), startIndex + 1, "index incremented");

        uint256 expectedHash = ShaHashChainLib.compute(previousHash, ALICE, BOB, transferAmount);
        assertEq(token.hashChain(), expectedHash, "hash chain chained");
    }

    function testTeleportRequiresVerifierAndMints() public {
        uint256 value = 2 ether;

        vm.expectRevert(zERC20.OnlyVerifier.selector);
        token.teleport(ALICE, value);

        token.setVerifier(address(this));

        vm.expectEmit(true, true, false, true, address(token));
        emit Teleport(ALICE, value);
        token.teleport(ALICE, value);

        assertEq(token.balanceOf(ALICE), value, "teleport balance");
        assertEq(token.totalSupply(), value, "supply after teleport");
        assertEq(token.index(), 1, "index after teleport");

        uint256 expectedHash = ShaHashChainLib.compute(0, address(0), ALICE, value);
        assertEq(token.hashChain(), expectedHash, "hash chain after teleport");
    }

    function testTeleportAccumulatesTotalTeleported() public {
        uint256 first = 1 ether;
        uint256 second = 4 ether;

        token.setVerifier(address(this));

        assertEq(token.totalTeleported(), 0, "initial total");

        token.teleport(ALICE, first);
        assertEq(token.totalTeleported(), first, "after first teleport");

        token.teleport(BOB, second);
        assertEq(token.totalTeleported(), first + second, "after second teleport");
    }

    function testPermitSetsAllowanceAndRespectsTypedData() public {
        uint256 ownerKey = 0xA11CE;
        address owner = vm.addr(ownerKey);
        uint256 value = 2 ether;
        uint256 deadline = block.timestamp + 1 days;

        token.mint(owner, value);
        uint256 hashAfterMint = token.hashChain();
        uint256 indexAfterMint = token.index();

        bytes32 structHash = keccak256(abi.encode(PERMIT_TYPEHASH, owner, BOB, value, token.nonces(owner), deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", token.DOMAIN_SEPARATOR(), structHash));
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ownerKey, digest);

        token.permit(owner, BOB, value, deadline, v, r, s);

        assertEq(token.allowance(owner, BOB), value, "permit allowance");
        assertEq(token.nonces(owner), 1, "nonce consumed");

        vm.prank(BOB);
        bool transferOk = token.transferFrom(owner, BOB, value);
        assertTrue(transferOk, "transferFrom should return true");

        assertEq(token.balanceOf(BOB), value, "transferred via permit");
        assertEq(token.index(), indexAfterMint + 1, "index incremented");
        uint256 expectedHash = ShaHashChainLib.compute(hashAfterMint, owner, BOB, value);
        assertEq(token.hashChain(), expectedHash, "hash chain after permit transfer");
    }

    function testPermitRejectsExpiredSignature() public {
        uint256 ownerKey = 0xBEEF;
        address owner = vm.addr(ownerKey);
        uint256 value = 1 ether;
        uint256 deadline = block.timestamp;
        if (deadline > 0) {
            --deadline;
        }

        bytes32 structHash = keccak256(abi.encode(PERMIT_TYPEHASH, owner, BOB, value, token.nonces(owner), deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", token.DOMAIN_SEPARATOR(), structHash));
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ownerKey, digest);

        vm.expectRevert(abi.encodeWithSelector(ERC20PermitUpgradeable.ERC2612ExpiredSignature.selector, deadline));
        token.permit(owner, BOB, value, deadline, v, r, s);
    }

    function testInitializeSupportsCustomDecimals() public {
        uint8 customDecimals = 8;
        zERC20 customToken = _deployToken(address(this), endpoint, customDecimals);
        assertEq(customToken.decimals(), customDecimals, "custom decimals stored");
        assertEq(
            customToken.decimalConversionRate(),
            10 ** (customDecimals - customToken.sharedDecimals()),
            "conversion rate uses custom decimals"
        );
    }

    function testInitializeRejectsBelowSharedDecimals() public {
        vm.expectRevert(IOFT.InvalidLocalDecimals.selector);
        new zERC20(address(endpoint), 5, IBlocklist(address(bl)));
    }

    function testMintOnlyMinter() public {
        vm.prank(ALICE);
        vm.expectRevert();
        token.mint(ALICE, 1 ether);

        token.setMinter(ALICE);
        vm.prank(ALICE);
        token.mint(ALICE, 4 ether);

        assertEq(token.minter(), ALICE, "minter updated");
        assertEq(token.balanceOf(ALICE), 4 ether, "minted by new minter");
    }

    function testBurnRequiresMinterAndUpdatesState() public {
        uint256 mintAmount = 8 ether;
        uint256 burnAmount = 3 ether;

        token.mint(ALICE, mintAmount);

        vm.prank(ALICE);
        vm.expectRevert();
        token.burn(ALICE, burnAmount);

        uint256 hashAfterMint = token.hashChain();
        uint256 indexAfterMint = token.index();
        uint256 supplyAfterMint = token.totalSupply();

        token.burn(ALICE, burnAmount);

        assertEq(token.balanceOf(ALICE), mintAmount - burnAmount, "balance after burn");
        assertEq(token.totalSupply(), supplyAfterMint - burnAmount, "supply after burn");
        assertEq(token.index(), indexAfterMint + 1, "index increment after burn");

        uint256 expectedHash = ShaHashChainLib.compute(hashAfterMint, ALICE, address(0), burnAmount);
        assertEq(token.hashChain(), expectedHash, "hash chain after burn");
    }

    function testSetVerifierRestrictedToOwner() public {
        address nonOwner = address(0xBEEF);
        address newVerifier = address(0x1234);

        vm.prank(nonOwner);
        vm.expectRevert(abi.encodeWithSelector(OwnableUpgradeable.OwnableUnauthorizedAccount.selector, nonOwner));
        token.setVerifier(newVerifier);

        vm.expectEmit(true, true, false, false, address(token));
        emit VerifierUpdated(newVerifier);
        token.setVerifier(newVerifier);
        assertEq(token.verifier(), newVerifier, "verifier stored");
    }

    function testSetVerifierRejectsZeroAddress() public {
        vm.expectRevert(zERC20.ZeroAddress.selector);
        token.setVerifier(address(0));
    }

    function testSetMinterRestrictedToOwner() public {
        address nonOwner = address(0xBEEF);
        address newMinter = address(0x1234);

        vm.prank(nonOwner);
        vm.expectRevert(abi.encodeWithSelector(OwnableUpgradeable.OwnableUnauthorizedAccount.selector, nonOwner));
        token.setMinter(newMinter);

        vm.expectEmit(true, false, false, false, address(token));
        emit MinterUpdated(newMinter);
        token.setMinter(newMinter);
        assertEq(token.minter(), newMinter, "minter stored");
    }

    function testSetMinterAllowsZeroAddress() public {
        // First set a non-zero minter
        token.setMinter(ALICE);
        assertEq(token.minter(), ALICE, "minter set to alice");

        // Then disable by setting to zero
        vm.expectEmit(true, false, false, false, address(token));
        emit MinterUpdated(address(0));
        token.setMinter(address(0));
        assertEq(token.minter(), address(0), "minter disabled");
    }

    function testMintFailsWhenMinterNotSet() public {
        // Deploy fresh token without setting minter
        ZERC20Harness freshToken = _deployToken(address(this), endpoint, 18);
        // minter is address(0) by default

        vm.expectRevert(zERC20.OnlyMinter.selector);
        freshToken.mint(ALICE, 1 ether);
    }

    function testBurnFailsWhenMinterNotSet() public {
        // Deploy fresh token, set minter, mint, then disable minter
        ZERC20Harness freshToken = _deployToken(address(this), endpoint, 18);
        freshToken.setMinter(address(this));
        freshToken.mint(ALICE, 10 ether);

        // Disable minter
        freshToken.setMinter(address(0));

        vm.expectRevert(zERC20.OnlyMinter.selector);
        freshToken.burn(ALICE, 1 ether);
    }

    function testValueTooLargeReverts() public {
        uint256 tooLarge = uint256(type(uint248).max) + 1;

        vm.expectRevert(zERC20.ValueTooLarge.selector);
        token.mint(ALICE, tooLarge);
    }

    function testRemoveDustRoundsDownToConversionRate() public view {
        uint256 conversionRate = token.decimalConversionRate();
        assertEq(token.removeDust(conversionRate - 1), 0, "dust below conversion rate");
        assertEq(token.removeDust(conversionRate + 123), conversionRate, "dust rounded down");
    }

    function testToSdToLdRoundTripTruncatesDust() public view {
        uint256 conversionRate = token.decimalConversionRate();
        uint256 amountWithDust = conversionRate * 5 + 1;
        uint64 amountSd = token.toSd(amountWithDust);
        assertEq(amountSd, 5, "toSd truncates dust");
        assertEq(token.toLd(amountSd), conversionRate * 5, "toLd restores dustless amount");
    }

    function testToSdRevertsOnOverflow() public {
        uint256 conversionRate = token.decimalConversionRate();
        uint256 amountSdOverflow = uint256(type(uint64).max) + 1;
        uint256 amountLdOverflow = amountSdOverflow * conversionRate;
        vm.expectRevert(abi.encodeWithSelector(IOFT.AmountSDOverflowed.selector, amountSdOverflow));
        token.toSd(amountLdOverflow);
    }

    function testDebitViewRevertsWhenMinAmountExceedsDustlessAmount() public {
        uint256 conversionRate = token.decimalConversionRate();
        uint256 amountWithDust = conversionRate + 1;
        vm.expectRevert(abi.encodeWithSelector(IOFT.SlippageExceeded.selector, conversionRate, amountWithDust));
        token.debitView(amountWithDust, amountWithDust, 1);
    }

    function testCreditRedirectsZeroAddressToDeadAddress() public {
        uint256 amount = 5 ether;
        address deadAddress = address(0xdead);

        // Credit to address(0) should redirect to 0xdead
        uint256 received = token.credit(address(0), amount, 1);

        assertEq(received, amount, "amount received");
        assertEq(token.balanceOf(deadAddress), amount, "balance at 0xdead");
        assertEq(token.balanceOf(address(0)), 0, "balance at 0x0 should be 0");

        // Verify hash chain includes 0xdead, not address(0)
        uint256 expectedHash = ShaHashChainLib.compute(0, address(0), deadAddress, amount);
        assertEq(token.hashChain(), expectedHash, "hash chain should use 0xdead as recipient");
    }

    function testCreditNormalAddressWorks() public {
        uint256 amount = 3 ether;

        uint256 received = token.credit(ALICE, amount, 1);

        assertEq(received, amount, "amount received");
        assertEq(token.balanceOf(ALICE), amount, "balance at alice");
    }

    function testTokenReturnsItself() public view {
        assertEq(token.token(), address(token), "token() should return self");
    }

    function testApprovalRequiredReturnsFalse() public view {
        assertFalse(token.approvalRequired(), "approvalRequired should be false");
    }

    function testDebitBurnsTokens() public {
        uint256 mintAmount = 10 ether;
        uint256 debitAmount = 3 ether;

        token.mint(ALICE, mintAmount);
        uint256 hashAfterMint = token.hashChain();
        uint256 indexAfterMint = token.index();

        vm.prank(ALICE);
        (uint256 amountDebited, uint256 amountToCredit) = token.debit(debitAmount, 0, 1);

        assertEq(amountDebited, debitAmount, "amount debited");
        assertEq(amountToCredit, debitAmount, "amount to credit");
        assertEq(token.balanceOf(ALICE), mintAmount - debitAmount, "balance after debit");
        assertEq(token.index(), indexAfterMint + 1, "index incremented");

        // Hash chain should include burn (to = address(0))
        uint256 expectedHash = ShaHashChainLib.compute(hashAfterMint, ALICE, address(0), debitAmount);
        assertEq(token.hashChain(), expectedHash, "hash chain after debit");
    }

    function testUpgradeRevertsOnEndpointMismatch() public {
        ZERC20Harness impl = new ZERC20Harness(address(endpoint), 18, IBlocklist(address(bl)));
        bytes memory initData = abi.encodeCall(zERC20.initialize, ("Test", "TST", address(this)));
        ERC1967Proxy proxy = new ERC1967Proxy(address(impl), initData);
        ZERC20Harness proxiedToken = ZERC20Harness(address(proxy));

        EndpointV2 otherEndpoint = new EndpointV2(2, address(this));
        ZERC20UpgradeMock newImpl = new ZERC20UpgradeMock(address(otherEndpoint), 18, IBlocklist(address(bl)));

        vm.expectRevert(
            abi.encodeWithSelector(zERC20.EndpointMismatch.selector, address(endpoint), address(otherEndpoint))
        );
        proxiedToken.upgradeToAndCall(address(newImpl), bytes(""));
    }

    // -----------------------------------------------------------------------
    // Blocklist
    // -----------------------------------------------------------------------

    function testBlocklistImmutableIsSet() public view {
        assertEq(address(token.BLOCKLIST()), address(bl), "blocklist immutable set");
    }

    function testBlockedAddressCannotTransfer() public {
        token.mint(ALICE, 10 ether);
        bl.blockAddress(ALICE);

        vm.prank(ALICE);
        vm.expectRevert(abi.encodeWithSelector(zERC20.AddressIsBlocked.selector, ALICE));
        token.transfer(BOB, 1 ether);
    }

    function testBlockedAddressCannotReceiveTransfer() public {
        token.mint(ALICE, 10 ether);
        bl.blockAddress(BOB);

        vm.prank(ALICE);
        vm.expectRevert(abi.encodeWithSelector(zERC20.AddressIsBlocked.selector, BOB));
        token.transfer(BOB, 1 ether);
    }

    function testBlockedAddressCannotReceiveMint() public {
        bl.blockAddress(ALICE);

        vm.expectRevert(abi.encodeWithSelector(zERC20.AddressIsBlocked.selector, ALICE));
        token.mint(ALICE, 1 ether);
    }

    function testBlockedAddressCannotBurn() public {
        token.mint(ALICE, 10 ether);
        bl.blockAddress(ALICE);

        vm.expectRevert(abi.encodeWithSelector(zERC20.AddressIsBlocked.selector, ALICE));
        token.burn(ALICE, 1 ether);
    }

    function testBlockedAddressCannotReceiveTeleport() public {
        token.setVerifier(address(this));
        bl.blockAddress(ALICE);

        vm.expectRevert(abi.encodeWithSelector(zERC20.AddressIsBlocked.selector, ALICE));
        token.teleport(ALICE, 1 ether);
    }

    function testUnblockedAddressCanTransferAgain() public {
        token.mint(ALICE, 10 ether);
        bl.blockAddress(ALICE);
        bl.unblockAddress(ALICE);

        vm.prank(ALICE);
        bool ok = token.transfer(BOB, 1 ether);
        assertTrue(ok, "transfer succeeds after unblock");
        assertEq(token.balanceOf(BOB), 1 ether, "bob received");
    }

    function testSharedBlocklistAcrossTokens() public {
        ZERC20Harness token2 = _deployToken(address(this), endpoint, 18);
        token2.setMinter(address(this));

        bl.blockAddress(ALICE);

        vm.expectRevert(abi.encodeWithSelector(zERC20.AddressIsBlocked.selector, ALICE));
        token.mint(ALICE, 1 ether);

        vm.expectRevert(abi.encodeWithSelector(zERC20.AddressIsBlocked.selector, ALICE));
        token2.mint(ALICE, 1 ether);
    }

    function testNonBlockedTransferSucceeds() public {
        token.mint(ALICE, 10 ether);
        vm.prank(ALICE);
        bool ok = token.transfer(BOB, 1 ether);
        assertTrue(ok, "non-blocked transfer succeeds");
    }

    function testBlockedAddressCannotTransferFrom() public {
        token.mint(ALICE, 10 ether);
        vm.prank(ALICE);
        token.approve(BOB, 5 ether);

        bl.blockAddress(ALICE);

        vm.prank(BOB);
        vm.expectRevert(abi.encodeWithSelector(zERC20.AddressIsBlocked.selector, ALICE));
        token.transferFrom(ALICE, BOB, 1 ether);
    }

    function testBlockedSpenderCannotReceiveViaTransferFrom() public {
        token.mint(ALICE, 10 ether);
        vm.prank(ALICE);
        token.approve(BOB, 5 ether);

        bl.blockAddress(BOB);

        vm.prank(BOB);
        vm.expectRevert(abi.encodeWithSelector(zERC20.AddressIsBlocked.selector, BOB));
        token.transferFrom(ALICE, BOB, 1 ether);
    }

    function testOftDebitBlockedSenderReverts() public {
        token.mint(ALICE, 10 ether);
        bl.blockAddress(ALICE);

        vm.prank(ALICE);
        vm.expectRevert(abi.encodeWithSelector(zERC20.AddressIsBlocked.selector, ALICE));
        token.debit(1 ether, 0, 1);
    }

    function testOftCreditToBlockedDeadAddressReverts() public {
        bl.blockAddress(address(0xdead));

        // _credit(address(0)) redirects to 0xdead, which is blocked
        vm.expectRevert(abi.encodeWithSelector(zERC20.AddressIsBlocked.selector, address(0xdead)));
        token.credit(address(0), 1 ether, 1);
    }

    function testUpgradeSucceedsWithSameEndpoint() public {
        ZERC20Harness impl = new ZERC20Harness(address(endpoint), 18, IBlocklist(address(bl)));
        bytes memory initData = abi.encodeCall(zERC20.initialize, ("Test", "TST", address(this)));
        ERC1967Proxy proxy = new ERC1967Proxy(address(impl), initData);
        ZERC20Harness proxiedToken = ZERC20Harness(address(proxy));

        // Mint some tokens to verify state preservation
        proxiedToken.setMinter(address(this));
        proxiedToken.mint(ALICE, 5 ether);

        ZERC20UpgradeMock newImpl = new ZERC20UpgradeMock(address(endpoint), 18, IBlocklist(address(bl)));
        proxiedToken.upgradeToAndCall(address(newImpl), bytes(""));

        ZERC20UpgradeMock upgraded = ZERC20UpgradeMock(address(proxiedToken));
        assertEq(upgraded.version(), "v2", "upgrade succeeded");
        assertEq(upgraded.balanceOf(ALICE), 5 ether, "state preserved");
    }

    /// @notice Simulates upgrading from a pre-blocklist impl to a blocklist-enabled impl.
    ///         Verifies all storage state is preserved and BLOCKLIST immutable is accessible.
    function testUpgradeToBlocklistImplPreservesState() public {
        // Deploy "old" impl and proxy (using a different blocklist to simulate pre-upgrade)
        Blocklist oldBl = new Blocklist(address(this));
        ZERC20Harness oldImpl = new ZERC20Harness(address(endpoint), 18, IBlocklist(address(oldBl)));
        bytes memory initData = abi.encodeCall(zERC20.initialize, ("Test", "TST", address(this)));
        ERC1967Proxy proxy = new ERC1967Proxy(address(oldImpl), initData);
        ZERC20Harness proxiedToken = ZERC20Harness(address(proxy));

        // Set up state: minter, verifier, mint tokens, do a transfer
        proxiedToken.setMinter(address(this));
        proxiedToken.setVerifier(address(0x1234));
        proxiedToken.mint(ALICE, 10 ether);
        vm.prank(ALICE);
        proxiedToken.transfer(BOB, 3 ether);

        // Snapshot pre-upgrade state
        uint256 aliceBalance = proxiedToken.balanceOf(ALICE);
        uint256 bobBalance = proxiedToken.balanceOf(BOB);
        uint256 totalSupply = proxiedToken.totalSupply();
        uint256 hashChainBefore = proxiedToken.hashChain();
        uint256 indexBefore = proxiedToken.index();
        address verifierBefore = proxiedToken.verifier();
        address minterBefore = proxiedToken.minter();

        // Upgrade to new impl with a different Blocklist
        ZERC20Harness newImpl = new ZERC20Harness(address(endpoint), 18, IBlocklist(address(bl)));
        proxiedToken.upgradeToAndCall(address(newImpl), bytes(""));

        // Verify BLOCKLIST immutable is accessible and correct
        assertEq(address(proxiedToken.BLOCKLIST()), address(bl), "BLOCKLIST immutable set after upgrade");

        // Verify all storage state is preserved
        assertEq(proxiedToken.balanceOf(ALICE), aliceBalance, "alice balance preserved");
        assertEq(proxiedToken.balanceOf(BOB), bobBalance, "bob balance preserved");
        assertEq(proxiedToken.totalSupply(), totalSupply, "total supply preserved");
        assertEq(proxiedToken.hashChain(), hashChainBefore, "hash chain preserved");
        assertEq(proxiedToken.index(), indexBefore, "index preserved");
        assertEq(proxiedToken.verifier(), verifierBefore, "verifier preserved");
        assertEq(proxiedToken.minter(), minterBefore, "minter preserved");

        // Verify blocklist enforcement works after upgrade
        bl.blockAddress(BOB);
        vm.prank(ALICE);
        vm.expectRevert(abi.encodeWithSelector(zERC20.AddressIsBlocked.selector, BOB));
        proxiedToken.transfer(BOB, 1 ether);

        // Verify non-blocked transfers still work
        vm.prank(ALICE);
        bool ok = proxiedToken.transfer(address(0xCAFE), 1 ether);
        assertTrue(ok, "non-blocked transfer works after upgrade");
        assertEq(proxiedToken.index(), indexBefore + 1, "index incremented after upgrade transfer");
    }
}

contract ZERC20UpgradeMock is zERC20 {
    constructor(address endpoint_, uint8 decimals_, IBlocklist blocklist_) zERC20(endpoint_, decimals_, blocklist_) {}

    function version() external pure returns (string memory) {
        return "v2";
    }
}
