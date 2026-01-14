// SPDX-License-Identifier: MIT
pragma solidity ^0.8.22;

import {Test, console} from "forge-std/Test.sol";
import {LZAddressContext} from "lz-address-book/helpers/LZAddressContext.sol";

contract Setting is Test {
    LZAddressContext ctx;

    function setUp() public {
        ctx = new LZAddressContext();
    }

    function test_getAddresses() public {
        // Set chain context (pick one method)
        // ctx.setChain("arbitrum-testnet"); // by name
        // ctx.setChainByEid(30110);            // by LayerZero EID
        ctx.setChainByChainId(421614);        // by native chain ID

        // Get addresses
        address endpoint = ctx.getEndpointV2();
        address sendLib = ctx.getSendUln302();
        address receiveLib = ctx.getReceiveUln302();
        address executor = ctx.getExecutor();
        address dvn = ctx.getDVNByName("LayerZero Labs");

        console.log("Endpoint:", endpoint);
        console.log("DVN:", dvn);

        // Verify
        assertEq(endpoint, 0x1a44076050125825900e736c501f859c50fE728c);
    }
}
