// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Test, console} from "forge-std/Test.sol";
import {
    EndpointV2Mock as EndpointV2
} from "@layerzerolabs/test-devtools-evm-foundry/contracts/mocks/EndpointV2Mock.sol";
import {zERC20} from "../src/zERC20.sol";
import {IBlocklist} from "../src/interfaces/IBlocklist.sol";
import {Blocklist} from "../src/Blocklist.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";

/// @dev Harness that exposes the blocklist check as it happens inside _update:
///      proxy → delegatecall → read immutable BLOCKLIST → STATICCALL Blocklist.isBlocked
contract BlocklistGasHarness is zERC20 {
    constructor(address endpoint, uint8 decimals_, IBlocklist blocklist_) zERC20(endpoint, decimals_, blocklist_) {}

    /// @notice Mirrors the exact code path of `_update`'s blocklist check.
    function checkBlocked(address account) external view returns (bool) {
        return BLOCKLIST.isBlocked(account);
    }
}

/// @title zERC20 Blocklist Gas Benchmarks (Immutable + External Contract)
/// @notice Measures gas for registration and transfer operations with 70 blocked addresses.
contract ZERC20BlocklistGasTest is Test {
    BlocklistGasHarness internal token;
    Blocklist internal bl;
    EndpointV2 internal endpoint;

    address internal constant ALICE = address(0xA11CE);
    address internal constant BOB = address(0xB0B);

    // 70 sanctioned addresses for testing
    address[] internal sanctionedAddresses;

    function setUp() public {
        endpoint = new EndpointV2(1, address(this));
        bl = new Blocklist(address(this));
        BlocklistGasHarness impl = new BlocklistGasHarness(address(endpoint), 18, IBlocklist(address(bl)));
        bytes memory initData = abi.encodeCall(zERC20.initialize, ("Zero Token", "ZTK", address(this)));
        ERC1967Proxy proxy = new ERC1967Proxy(address(impl), initData);
        token = BlocklistGasHarness(address(proxy));
        token.setMinter(address(this));

        // Generate 70 unique addresses
        for (uint256 i = 1; i <= 70; ++i) {
            sanctionedAddresses.push(address(uint160(0xDEAD0000 + i)));
        }
    }

    // -----------------------------------------------------------------------
    // Registration gas
    // -----------------------------------------------------------------------

    /// @notice Gas to register 70 addresses one-by-one on Blocklist contract
    function testGas_blockAddress_70_individual() public {
        uint256 gasBefore = gasleft();
        for (uint256 i; i < 70; ++i) {
            bl.blockAddress(sanctionedAddresses[i]);
        }
        uint256 gasUsed = gasBefore - gasleft();
        console.log("[immutable-external] blockAddress x70 individual:", gasUsed);
    }

    /// @notice Gas to register 70 addresses via batch on Blocklist contract
    function testGas_blockAddresses_70_batch() public {
        uint256 gasBefore = gasleft();
        bl.blockAddresses(sanctionedAddresses);
        uint256 gasUsed = gasBefore - gasleft();
        console.log("[immutable-external] blockAddresses x70 batch:", gasUsed);
    }

    // -----------------------------------------------------------------------
    // Transfer gas (0 blocked addresses registered)
    // -----------------------------------------------------------------------

    /// @notice Transfer with 0 blocked addresses (blocklist is set but empty)
    function testGas_transfer_0Blocked() public {
        token.mint(ALICE, 100 ether);

        vm.prank(ALICE);
        uint256 gasBefore = gasleft();
        token.transfer(BOB, 1 ether);
        uint256 gasUsed = gasBefore - gasleft();
        console.log("[immutable-external] transfer (0 blocked):", gasUsed);
    }

    // -----------------------------------------------------------------------
    // Transfer gas (70 blocked addresses registered)
    // -----------------------------------------------------------------------

    /// @notice Transfer with 70 blocked addresses registered (non-blocked sender/receiver)
    function testGas_transfer_with70Blocked() public {
        bl.blockAddresses(sanctionedAddresses);
        token.mint(ALICE, 100 ether);

        vm.prank(ALICE);
        uint256 gasBefore = gasleft();
        token.transfer(BOB, 1 ether);
        uint256 gasUsed = gasBefore - gasleft();
        console.log("[immutable-external] transfer (70 blocked, non-blocked user):", gasUsed);
    }

    /// @notice Transfer where sender is blocked (expect revert)
    function testGas_transfer_blockedSender() public {
        address blocked = sanctionedAddresses[35]; // middle of the list
        token.mint(blocked, 100 ether);
        bl.blockAddresses(sanctionedAddresses);

        vm.prank(blocked);
        vm.expectRevert(abi.encodeWithSelector(zERC20.AddressIsBlocked.selector, blocked));
        token.transfer(BOB, 1 ether);
    }

    // -----------------------------------------------------------------------
    // Mint gas
    // -----------------------------------------------------------------------

    /// @notice Mint with 0 blocked addresses
    function testGas_mint_0Blocked() public {
        uint256 gasBefore = gasleft();
        token.mint(ALICE, 1 ether);
        uint256 gasUsed = gasBefore - gasleft();
        console.log("[immutable-external] mint (0 blocked):", gasUsed);
    }

    /// @notice Mint with 70 blocked addresses registered (non-blocked recipient)
    function testGas_mint_with70Blocked() public {
        bl.blockAddresses(sanctionedAddresses);

        uint256 gasBefore = gasleft();
        token.mint(ALICE, 1 ether);
        uint256 gasUsed = gasBefore - gasleft();
        console.log("[immutable-external] mint (70 blocked, non-blocked user):", gasUsed);
    }

    // -----------------------------------------------------------------------
    // isBlocked query gas (via proxy, same call path as _update)
    // -----------------------------------------------------------------------

    /// @notice isBlocked for a blocked address — called through proxy → immutable read → STATICCALL
    function testGas_isBlocked_true() public {
        bl.blockAddresses(sanctionedAddresses);

        uint256 gasBefore = gasleft();
        token.checkBlocked(sanctionedAddresses[0]);
        uint256 gasUsed = gasBefore - gasleft();
        console.log("[immutable-external] isBlocked via proxy (true):", gasUsed);
    }

    /// @notice isBlocked for a non-blocked address — called through proxy → immutable read → STATICCALL
    function testGas_isBlocked_false() public {
        bl.blockAddresses(sanctionedAddresses);

        uint256 gasBefore = gasleft();
        token.checkBlocked(ALICE);
        uint256 gasUsed = gasBefore - gasleft();
        console.log("[immutable-external] isBlocked via proxy (false):", gasUsed);
    }

    // -----------------------------------------------------------------------
    // Multi-token registration cost (simulating 3 tokens on one chain)
    // -----------------------------------------------------------------------

    /// @notice Register 70 addresses once on shared Blocklist (constructor already binds it)
    function testGas_blockAddresses_3tokens() public {
        uint256 gasBefore = gasleft();
        bl.blockAddresses(sanctionedAddresses);
        uint256 gasUsed = gasBefore - gasleft();
        console.log("[immutable-external] blockAddresses x70 (shared, 1 tx for all tokens):", gasUsed);
    }
}
