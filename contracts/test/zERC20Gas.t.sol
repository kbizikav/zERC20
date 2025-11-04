// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {zERC20} from "../src/zERC20.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";

/// @notice Compares transfer gas costs between the custom zERC20 and a baseline ERC20.
contract zERC20GasTest is Test {
    zERC20 internal zToken;
    SimpleERC20 internal standardToken;

    uint256 internal constant INITIAL_SUPPLY = 1_000_000 ether;
    uint256 internal constant TRANSFER_AMOUNT = 1 ether;

    address internal constant Z_RECIPIENT = address(0xBEEF);
    address internal constant STANDARD_RECIPIENT = address(0xBEE0);

    function setUp() public {
        zERC20 impl = new zERC20();
        bytes memory initData = abi.encodeCall(zERC20.initialize, ("Zero Token", "ZTK", address(this)));
        ERC1967Proxy proxy = new ERC1967Proxy(address(impl), initData);
        zToken = zERC20(address(proxy));
        zToken.setMinter(address(this));
        zToken.mint(address(this), INITIAL_SUPPLY);

        standardToken = new SimpleERC20("Standard Token", "STK", INITIAL_SUPPLY);
    }

    function testTransferGasComparison() public {
        uint256 gasBeforeStandard = gasleft();
        bool standardTransferOk = standardToken.transfer(STANDARD_RECIPIENT, TRANSFER_AMOUNT);
        assertTrue(standardTransferOk, "standard transfer should succeed");
        uint256 gasAfterStandard = gasleft();
        uint256 gasStandard = gasBeforeStandard - gasAfterStandard;

        uint256 gasBeforeZ = gasleft();
        bool zTransferOk = zToken.transfer(Z_RECIPIENT, TRANSFER_AMOUNT);
        assertTrue(zTransferOk, "zERC20 transfer should succeed");
        uint256 gasAfterZ = gasleft();
        uint256 gasZ = gasBeforeZ - gasAfterZ;

        emit log_named_uint("standard transfer gas", gasStandard);
        emit log_named_uint("zERC20 transfer gas", gasZ);

        assertGt(gasZ, gasStandard, "zERC20 transfer gas should exceed baseline ERC20 transfer gas");
    }
}

/// @notice Minimal ERC20 used to benchmark against zERC20.
contract SimpleERC20 is ERC20 {
    constructor(string memory name_, string memory symbol_, uint256 supply_) ERC20(name_, symbol_) {
        _mint(msg.sender, supply_);
    }
}
