// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {EndpointV2Mock as EndpointV2} from "@layerzerolabs/test-devtools-evm-foundry/contracts/mocks/EndpointV2Mock.sol";
import {zERC20} from "../src/zERC20.sol";
import {ShaHashChainLib} from "../src/utils/ShaHashChainLib.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {IOFT, SendParam} from "@layerzerolabs/oft-evm/contracts/interfaces/IOFT.sol";

contract ZERC20Harness is zERC20 {
    constructor(address endpoint, uint8 decimals_) zERC20(endpoint, decimals_) {}

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
    EndpointV2 internal endpoint;

    address internal constant ALICE = address(0xA11CE);
    address internal constant BOB = address(0xB0B);

    bytes32 internal constant PERMIT_TYPEHASH =
        keccak256("Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)");

    event IndexedTransfer(uint256 indexed index, address from, address to, uint256 value);
    event Teleport(address indexed to, uint256 value);
    event VerifierUpdated(address indexed newVerifier);

    function setUp() public {
        endpoint = new EndpointV2(1, address(this));
        token = _deployToken(address(this), endpoint, 18);
        token.setMinter(address(this));
    }

    function _deployToken(address owner, EndpointV2 endpointMock, uint8 decimals_) private returns (ZERC20Harness) {
        ZERC20Harness impl = new ZERC20Harness(address(endpointMock), decimals_);
        bytes memory initData = abi.encodeCall(zERC20.initialize, ("Zero Token", "ZTK", owner));
        ERC1967Proxy proxy = new ERC1967Proxy(address(impl), initData);
        return ZERC20Harness(address(proxy));
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

        vm.expectRevert();
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

        bytes32 structHash = keccak256(abi.encode(PERMIT_TYPEHASH, owner, BOB, value, token.nonces(owner), deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", token.DOMAIN_SEPARATOR(), structHash));
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ownerKey, digest);

        vm.warp(deadline + 1);
        vm.expectRevert("ERC20Permit: expired deadline");
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
        new zERC20(address(endpoint), 5);
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
        vm.expectRevert("Ownable: caller is not the owner");
        token.setVerifier(newVerifier);

        vm.expectEmit(true, true, false, false, address(token));
        emit VerifierUpdated(newVerifier);
        token.setVerifier(newVerifier);
        assertEq(token.verifier(), newVerifier, "verifier stored");
    }
}
