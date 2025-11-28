// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {TestHelperOz5, EndpointV2Mock} from "./utils/TestHelperOz5.sol";
import {zERC20} from "../src/zERC20.sol";
import {ShaHashChainLib} from "../src/utils/ShaHashChainLib.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";

contract ZERC20Test is TestHelperOz5 {
    zERC20 internal token;
    EndpointV2Mock internal endpoint;

    address internal constant ALICE = address(0xA11CE);
    address internal constant BOB = address(0xB0B);

    event IndexedTransfer(uint256 indexed index, address from, address to, uint256 value);
    event Teleport(address indexed to, uint256 value);
    event VerifierUpdated(address indexed newVerifier);

    function setUp() public {
        endpoint = _deployEndpoint(101);
        token = _deployToken(address(this), endpoint, 18);
        token.setMinter(address(this));
    }

    function _deployToken(address owner, EndpointV2Mock endpointMock, uint8 decimals_) private returns (zERC20) {
        zERC20 impl = new zERC20();
        bytes memory initData =
            abi.encodeCall(zERC20.initialize, ("Zero Token", "ZTK", owner, address(endpointMock), decimals_));
        ERC1967Proxy proxy = new ERC1967Proxy(address(impl), initData);
        return zERC20(address(proxy));
    }

    function testMintInitializesHashChainAndIndex() public {
        uint256 amount = 5 ether;

        vm.expectEmit(true, true, false, true, address(token));
        emit IndexedTransfer(0, address(0), ALICE, amount);
        token.mint(ALICE, amount);

        assertEq(token.balanceOf(ALICE), amount, "minted balance");
        assertEq(token.index(), 1, "index after mint");

        uint256 expectedHash = ShaHashChainLib.compute(0, ALICE, amount);
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

        uint256 expectedHash = ShaHashChainLib.compute(previousHash, BOB, transferAmount);
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

        uint256 expectedHash = ShaHashChainLib.compute(0, ALICE, value);
        assertEq(token.hashChain(), expectedHash, "hash chain after teleport");
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
        zERC20 impl = new zERC20();
        vm.expectRevert(zERC20.InvalidDecimals.selector);
        new ERC1967Proxy(
            address(impl), abi.encodeCall(zERC20.initialize, ("Zero Token", "ZTK", address(this), address(endpoint), 5))
        );
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

        uint256 expectedHash = ShaHashChainLib.compute(hashAfterMint, address(0), burnAmount);
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
