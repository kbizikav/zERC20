// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.33;

import {Test} from "forge-std/Test.sol";
import {SlotDerivation} from "@openzeppelin/contracts/utils/SlotDerivation.sol";
import {Adaptor} from "../src/liquidity/Adaptor.sol";
import {LiquidityManager} from "../src/liquidity/LiquidityManager.sol";
import {Hub} from "../src/Hub.sol";
import {Verifier} from "../src/Verifier.sol";
import {zERC20} from "../src/zERC20.sol";
import {IBlocklist} from "../src/interfaces/IBlocklist.sol";
import {Blocklist} from "../src/Blocklist.sol";
import {SelfCall} from "../src/utils/SelfCall.sol";

contract MockLiquidityManager {
    address private immutable UNDERLYING;
    address private immutable ZERC20_TOKEN;

    constructor(address underlying_, address zerc20_) {
        UNDERLYING = underlying_;
        ZERC20_TOKEN = zerc20_;
    }

    function underlyingToken() external view returns (address) {
        return UNDERLYING;
    }

    function zerc20() external view returns (address) {
        return ZERC20_TOKEN;
    }
}

contract MockStargate {
    address private immutable TOKEN;

    constructor(address token_) {
        TOKEN = token_;
    }

    function token() external view returns (address) {
        return TOKEN;
    }

    function sharedDecimals() external pure returns (uint8) {
        return 6;
    }
}

contract AdaptorSlotHarness is Adaptor {
    constructor(address manager, address stargate, address lzEndpoint) Adaptor(manager, stargate, lzEndpoint) {}

    function slot() external pure returns (bytes32) {
        return ADAPTOR_STORAGE_SLOT;
    }
}

contract LiquidityManagerSlotHarness is LiquidityManager {
    constructor() LiquidityManager(address(0x1111), address(0x2222)) {}

    function slot() external pure returns (bytes32) {
        return LIQUIDITY_MANAGER_STORAGE_SLOT;
    }
}

contract HubSlotHarness is Hub {
    constructor(address endpoint) Hub(endpoint) {}

    function slot() external pure returns (bytes32) {
        return HUB_STORAGE_SLOT;
    }
}

contract VerifierSlotHarness is Verifier {
    constructor(address endpoint) Verifier(endpoint) {}

    function slot() external pure returns (bytes32) {
        return VERIFIER_STORAGE_SLOT;
    }
}

contract ZERC20SlotHarness is zERC20 {
    constructor(address endpoint, uint8 decimals_, IBlocklist blocklist_) zERC20(endpoint, decimals_, blocklist_) {}

    function slot() external pure returns (bytes32) {
        return ZERC20_STORAGE_SLOT;
    }
}

contract SelfCallSlotHarness is SelfCall {
    function slot() external pure returns (bytes32) {
        return SELF_CALL_STORAGE;
    }
}

contract StorageSlotDerivationTest is Test {
    function testAdaptorSlotConstantMatchesDerivation() public {
        MockLiquidityManager manager = new MockLiquidityManager(address(0x1111), address(0x2222));
        MockStargate stargate = new MockStargate(address(0x1111)); // token must match underlying
        AdaptorSlotHarness harness = new AdaptorSlotHarness(address(manager), address(stargate), address(0x4444));
        bytes32 expected = SlotDerivation.erc7201Slot("zerc20.storage.adaptor");
        assertEq(harness.slot(), expected, "adaptor slot");
    }

    function testLiquidityManagerSlotConstantMatchesDerivation() public {
        LiquidityManagerSlotHarness harness = new LiquidityManagerSlotHarness();
        bytes32 expected = SlotDerivation.erc7201Slot("zerc20.storage.liquidityManager");
        assertEq(harness.slot(), expected, "liquidity manager slot");
    }

    function testHubSlotConstantMatchesDerivation() public {
        HubSlotHarness harness = new HubSlotHarness(address(1));
        bytes32 expected = SlotDerivation.erc7201Slot("zerc20.storage.hub");
        assertEq(harness.slot(), expected, "hub slot");
    }

    function testVerifierSlotConstantMatchesDerivation() public {
        VerifierSlotHarness harness = new VerifierSlotHarness(address(1));
        bytes32 expected = SlotDerivation.erc7201Slot("zerc20.storage.verifier");
        assertEq(harness.slot(), expected, "verifier slot");
    }

    function testZERC20SlotConstantMatchesDerivation() public {
        Blocklist bl = new Blocklist(address(this));
        ZERC20SlotHarness harness = new ZERC20SlotHarness(address(1), 18, IBlocklist(address(bl)));
        bytes32 expected = SlotDerivation.erc7201Slot("zerc20.storage.zerc20");
        assertEq(harness.slot(), expected, "zerc20 slot");
    }

    function testSelfCallSlotConstantMatchesDerivation() public {
        SelfCallSlotHarness harness = new SelfCallSlotHarness();
        bytes32 expected = SlotDerivation.erc7201Slot("zerc20.storage.SelfCall");
        assertEq(harness.slot(), expected, "selfcall slot");
    }
}
