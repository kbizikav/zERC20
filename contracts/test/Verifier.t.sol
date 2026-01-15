// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Vm} from "forge-std/Vm.sol";
import {
    MessagingFee,
    MessagingReceipt
} from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/ILayerZeroEndpointV2.sol";
import {Origin} from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/ILayerZeroReceiver.sol";
import {IOAppCore} from "@layerzerolabs/oapp-evm/contracts/oapp/interfaces/IOAppCore.sol";
import {Verifier} from "../src/Verifier.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {
    TestHelperOz5,
    EndpointV2,
    SimpleMessageLibMock
} from "@layerzerolabs/test-devtools-evm-foundry/contracts/TestHelperOz5.sol";
import {GeneralRecipientLib} from "../src/utils/GeneralRecipientLib.sol";
import {IWithdrawDecider} from "../src/interfaces/IDecider.sol";

contract VerifierTest is TestHelperOz5 {
    Verifier internal verifier;
    EndpointV2 internal endpoint;
    SimpleMessageLibMock internal sendLib;

    address internal constant TOKEN = address(0xdead);
    address internal constant ROOT_DECIDER = address(0x100);
    address internal constant WITHDRAW_GLOBAL_DECIDER = address(0x200);
    address internal constant WITHDRAW_LOCAL_DECIDER = address(0x300);
    address internal constant SINGLE_WITHDRAW_GLOBAL_VERIFIER = address(0x400);
    address internal constant SINGLE_WITHDRAW_LOCAL_VERIFIER = address(0x500);

    uint32 internal constant LOCAL_EID = 1;
    uint32 internal constant HUB_EID = 2;
    uint256 internal constant FEE_PER_MESSAGE = 0.01 ether;

    bytes32 internal constant PACKET_SENT_SIG = keccak256("PacketSent(bytes,bytes,address)");

    function setUp() public override {
        super.setUp();
        setUpEndpoints(2, LibraryType.SimpleMessageLib);
        endpoint = endpointSetup.endpointList[0];
        sendLib = SimpleMessageLibMock(payable(endpointSetup.sendLibs[0]));
        sendLib.setMessagingFee(FEE_PER_MESSAGE, 0);

        verifier = _deployVerifier(address(this));
        verifier.setPeer(HUB_EID, _toBytes32(address(this)));
    }

    function testConstructorRevertsOnZeroEndpoint() public {
        vm.expectRevert(IOAppCore.InvalidEndpointCall.selector);
        new Verifier(address(0));
    }

    function testInitializeRevertsOnZeroToken() public {
        Verifier impl = new Verifier(address(endpoint));
        bytes memory initData = abi.encodeCall(
            Verifier.initialize,
            (
                address(0), // zero token
                HUB_EID,
                address(this),
                ROOT_DECIDER,
                WITHDRAW_GLOBAL_DECIDER,
                WITHDRAW_LOCAL_DECIDER,
                SINGLE_WITHDRAW_GLOBAL_VERIFIER,
                SINGLE_WITHDRAW_LOCAL_VERIFIER
            )
        );

        vm.expectRevert(Verifier.ZeroToken.selector);
        new ERC1967Proxy(address(impl), initData);
    }

    function testInitializeRevertsOnZeroDeciders() public {
        Verifier impl = new Verifier(address(endpoint));
        bytes memory initData = abi.encodeCall(
            Verifier.initialize,
            (
                TOKEN,
                HUB_EID,
                address(this),
                address(0), // zero root decider
                WITHDRAW_GLOBAL_DECIDER,
                WITHDRAW_LOCAL_DECIDER,
                SINGLE_WITHDRAW_GLOBAL_VERIFIER,
                SINGLE_WITHDRAW_LOCAL_VERIFIER
            )
        );

        vm.expectRevert(Verifier.ZeroAddress.selector);
        new ERC1967Proxy(address(impl), initData);
    }

    function testConstructorInitializes() public view {
        assertEq(verifier.token(), TOKEN, "token mismatch");
        assertEq(verifier.hubEid(), HUB_EID, "hub EID mismatch");
        assertEq(address(verifier.endpoint()), address(endpoint), "endpoint mismatch");
        assertEq(verifier.owner(), address(this), "owner mismatch");
        assertEq(verifier.rootDecider(), ROOT_DECIDER, "root decider mismatch");
        assertEq(verifier.withdrawGlobalDecider(), WITHDRAW_GLOBAL_DECIDER, "withdraw global decider mismatch");
        assertEq(verifier.withdrawLocalDecider(), WITHDRAW_LOCAL_DECIDER, "withdraw local decider mismatch");
        assertEq(
            verifier.singleWithdrawGlobalVerifier(),
            SINGLE_WITHDRAW_GLOBAL_VERIFIER,
            "single withdraw global verifier mismatch"
        );
        assertEq(
            verifier.singleWithdrawLocalVerifier(),
            SINGLE_WITHDRAW_LOCAL_VERIFIER,
            "single withdraw local verifier mismatch"
        );
    }

    function testSetVerifiers() public {
        address newRootDecider = address(0x600);
        address newWithdrawGlobalDecider = address(0x700);
        address newWithdrawLocalDecider = address(0x800);
        address newSingleWithdrawGlobalVerifier = address(0x900);
        address newSingleWithdrawLocalVerifier = address(0xa00);

        verifier.setVerifiers(
            newRootDecider,
            newWithdrawGlobalDecider,
            newWithdrawLocalDecider,
            newSingleWithdrawGlobalVerifier,
            newSingleWithdrawLocalVerifier
        );

        assertEq(verifier.rootDecider(), newRootDecider, "root decider not updated");
        assertEq(verifier.withdrawGlobalDecider(), newWithdrawGlobalDecider, "withdraw global decider not updated");
        assertEq(verifier.withdrawLocalDecider(), newWithdrawLocalDecider, "withdraw local decider not updated");
        assertEq(
            verifier.singleWithdrawGlobalVerifier(),
            newSingleWithdrawGlobalVerifier,
            "single withdraw global verifier not updated"
        );
        assertEq(
            verifier.singleWithdrawLocalVerifier(),
            newSingleWithdrawLocalVerifier,
            "single withdraw local verifier not updated"
        );
    }

    function testSetVerifiersZeroRootReverts() public {
        vm.expectRevert(Verifier.ZeroAddress.selector);
        verifier.setVerifiers(
            address(0),
            WITHDRAW_GLOBAL_DECIDER,
            WITHDRAW_LOCAL_DECIDER,
            SINGLE_WITHDRAW_GLOBAL_VERIFIER,
            SINGLE_WITHDRAW_LOCAL_VERIFIER
        );
    }

    function testSetVerifiersZeroSingleWithdrawGlobalReverts() public {
        vm.expectRevert(Verifier.ZeroAddress.selector);
        verifier.setVerifiers(
            ROOT_DECIDER, WITHDRAW_GLOBAL_DECIDER, WITHDRAW_LOCAL_DECIDER, address(0), SINGLE_WITHDRAW_LOCAL_VERIFIER
        );
    }

    function testSetVerifiersZeroSingleWithdrawLocalReverts() public {
        vm.expectRevert(Verifier.ZeroAddress.selector);
        verifier.setVerifiers(
            ROOT_DECIDER, WITHDRAW_GLOBAL_DECIDER, WITHDRAW_LOCAL_DECIDER, SINGLE_WITHDRAW_GLOBAL_VERIFIER, address(0)
        );
    }

    function testActivateEmergencyPausesContract() public {
        verifier.activateEmergency();
        assertTrue(verifier.paused(), "verifier should be paused");
    }

    function testActivateEmergencyOnlyOwner() public {
        address nonOwner = address(0xBEEF);
        vm.prank(nonOwner);
        vm.expectRevert(abi.encodeWithSelector(OwnableUpgradeable.OwnableUnauthorizedAccount.selector, nonOwner));
        verifier.activateEmergency();
    }

    function testActivateEmergencyCannotRunTwice() public {
        verifier.activateEmergency();
        vm.expectRevert(PausableUpgradeable.EnforcedPause.selector);
        verifier.activateEmergency();
    }

    function testRelayTransferRootSendsToEndpoint() public {
        uint256 root = verifier.provedTransferRoots(0);

        vm.deal(address(this), FEE_PER_MESSAGE);
        vm.recordLogs();
        MessagingReceipt memory receipt = verifier.relayTransferRoot{value: FEE_PER_MESSAGE}(bytes(""));

        assertEq(receipt.nonce, 1, "receipt nonce mismatch");
        assertEq(receipt.fee.nativeFee, FEE_PER_MESSAGE, "native fee mismatch");
        assertEq(receipt.fee.lzTokenFee, 0, "lz token fee mismatch");

        Vm.Log[] memory logs = vm.getRecordedLogs();
        bool foundTransferEvent;
        bool foundPacketSent;
        bytes32 transferSig = keccak256("TransferRootRelayed(uint64,uint256,bytes)");
        for (uint256 i = 0; i < logs.length; ++i) {
            if (logs[i].topics.length == 0) continue;
            if (logs[i].topics[0] == transferSig) {
                uint64 index = uint64(uint256(logs[i].topics[1]));
                (uint256 loggedRoot, bytes memory lzMsgId) = abi.decode(logs[i].data, (uint256, bytes));
                assertEq(index, 0, "index mismatch");
                assertEq(loggedRoot, root, "root mismatch");
                assertEq(_bytesToBytes32(lzMsgId), receipt.guid, "guid mismatch");
                foundTransferEvent = true;
            } else if (logs[i].topics[0] == PACKET_SENT_SIG) {
                (,, address lib) = abi.decode(logs[i].data, (bytes, bytes, address));
                assertEq(lib, address(sendLib), "send library mismatch");
                foundPacketSent = true;
            }
        }
        assertTrue(foundTransferEvent, "TransferRootRelayed not emitted");
        assertTrue(foundPacketSent, "PacketSent not emitted");
    }

    function testQuoteRelayUsesEndpointQuote() public view {
        MessagingFee memory fee = verifier.quoteRelay(bytes(""));
        assertEq(fee.nativeFee, FEE_PER_MESSAGE, "native fee mismatch");
        assertEq(fee.lzTokenFee, 0, "lz token fee mismatch");
    }

    function testLzReceiveStoresGlobalRoot() public {
        Origin memory origin = Origin({srcEid: HUB_EID, sender: _toBytes32(address(this)), nonce: 1});
        bytes memory payload = abi.encode(uint256(777), uint64(5));

        vm.recordLogs();
        vm.prank(address(endpoint));
        verifier.lzReceive(origin, bytes32(uint256(1)), payload, address(0xbabe), bytes(""));

        assertEq(verifier.globalTransferRoots(5), 777, "global root mismatch");
        assertEq(verifier.latestAggSeq(), 5, "latest agg seq mismatch");

        Vm.Log[] memory logs = vm.getRecordedLogs();
        bytes32 eventSig = keccak256("GlobalRootSaved(uint64,uint256)");
        bool found;
        for (uint256 i = 0; i < logs.length; ++i) {
            if (logs[i].topics.length > 0 && logs[i].topics[0] == eventSig) {
                uint64 seq = uint64(uint256(logs[i].topics[1]));
                uint256 root = abi.decode(logs[i].data, (uint256));
                assertEq(seq, 5, "agg seq mismatch");
                assertEq(root, 777, "root mismatch");
                found = true;
            }
        }
        assertTrue(found, "GlobalRootSaved not emitted");
    }

    function testVerifierUpgradePreservesState() public {
        Verifier implementation = new Verifier(address(endpoint));
        bytes memory initData = abi.encodeCall(
            Verifier.initialize,
            (
                TOKEN,
                HUB_EID,
                address(this),
                ROOT_DECIDER,
                WITHDRAW_GLOBAL_DECIDER,
                WITHDRAW_LOCAL_DECIDER,
                SINGLE_WITHDRAW_GLOBAL_VERIFIER,
                SINGLE_WITHDRAW_LOCAL_VERIFIER
            )
        );
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        Verifier proxiedVerifier = Verifier(address(proxy));

        address updatedRootDecider = address(0x1100);
        address updatedWithdrawGlobal = address(0x1200);
        address updatedWithdrawLocal = address(0x1300);
        address updatedSingleWithdrawGlobal = address(0x1400);
        address updatedSingleWithdrawLocal = address(0x1500);

        proxiedVerifier.setVerifiers(
            updatedRootDecider,
            updatedWithdrawGlobal,
            updatedWithdrawLocal,
            updatedSingleWithdrawGlobal,
            updatedSingleWithdrawLocal
        );

        VerifierUpgradeMock newImplementation = new VerifierUpgradeMock(address(endpoint));
        proxiedVerifier.upgradeToAndCall(address(newImplementation), bytes(""));

        VerifierUpgradeMock upgraded = VerifierUpgradeMock(address(proxiedVerifier));
        assertEq(upgraded.version(), "verifier-v2", "upgraded implementation not active");
        assertEq(proxiedVerifier.rootDecider(), updatedRootDecider, "root decider not preserved");
        assertEq(proxiedVerifier.withdrawGlobalDecider(), updatedWithdrawGlobal, "withdraw global not preserved");
        assertEq(proxiedVerifier.withdrawLocalDecider(), updatedWithdrawLocal, "withdraw local not preserved");
        assertEq(
            proxiedVerifier.singleWithdrawGlobalVerifier(),
            updatedSingleWithdrawGlobal,
            "single withdraw global not preserved"
        );
        assertEq(
            proxiedVerifier.singleWithdrawLocalVerifier(),
            updatedSingleWithdrawLocal,
            "single withdraw local not preserved"
        );
    }

    function _toBytes32(address addr) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(addr)));
    }

    function _deployVerifier(address delegate) internal returns (Verifier) {
        Verifier implementation = new Verifier(address(endpoint));
        bytes memory initData = abi.encodeCall(
            Verifier.initialize,
            (
                TOKEN,
                HUB_EID,
                delegate,
                ROOT_DECIDER,
                WITHDRAW_GLOBAL_DECIDER,
                WITHDRAW_LOCAL_DECIDER,
                SINGLE_WITHDRAW_GLOBAL_VERIFIER,
                SINGLE_WITHDRAW_LOCAL_VERIFIER
            )
        );
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        return Verifier(address(proxy));
    }

    function _bytesToBytes32(bytes memory data) internal pure returns (bytes32 value) {
        assertEq(data.length, 32, "guid length mismatch");
        // solhint-disable-next-line no-inline-assembly
        assembly {
            value := mload(add(data, 32))
        }
    }
}

contract VerifierUpgradeMock is Verifier {
    constructor(address endpoint) Verifier(endpoint) {}

    function version() external pure returns (string memory) {
        return "verifier-v2";
    }
}

// ============================================================================
// Reentrancy Test
// ============================================================================

/// @dev Mock zERC20 that triggers a callback during teleport
contract MockZERC20WithCallback {
    address public callbackTarget;
    bytes public callbackData;

    function setCallback(address target, bytes calldata data) external {
        callbackTarget = target;
        callbackData = data;
    }

    function teleport(address, uint256) external {
        if (callbackTarget != address(0)) {
            // Trigger callback before completing teleport
            /* solhint-disable-next-line avoid-low-level-calls */
            (bool success,) = callbackTarget.call(callbackData);
            // We don't care if it succeeds - we just want to attempt reentrancy
            success;
        }
    }
}

/// @dev Mock decider that always returns true
contract MockWithdrawDecider is IWithdrawDecider {
    function verifyOpaqueNovaProof(uint256[34] calldata) external pure returns (bool) {
        return true;
    }
}

/// @dev Attacker contract that attempts reentrancy
contract ReentrancyAttacker {
    Verifier public verifier;
    bool public attackAttempted;
    bool public attackSucceeded;

    constructor(Verifier _verifier) {
        verifier = _verifier;
    }

    function attack(
        bool isGlobal,
        uint64 rootHint,
        GeneralRecipientLib.GeneralRecipient calldata gr,
        bytes calldata proof
    ) external {
        attackAttempted = true;
        try verifier.teleport(isGlobal, rootHint, gr, proof) {
            attackSucceeded = true;
        } catch {
            attackSucceeded = false;
        }
    }
}

contract VerifierReentrancyTest is TestHelperOz5 {
    Verifier internal verifier;
    EndpointV2 internal endpoint;
    MockZERC20WithCallback internal mockToken;
    MockWithdrawDecider internal mockDecider;
    ReentrancyAttacker internal attacker;

    uint32 internal constant LOCAL_EID = 1;
    uint32 internal constant HUB_EID = 2;

    function setUp() public override {
        super.setUp();
        setUpEndpoints(2, LibraryType.SimpleMessageLib);
        endpoint = endpointSetup.endpointList[0];

        mockToken = new MockZERC20WithCallback();
        mockDecider = new MockWithdrawDecider();

        // Deploy verifier with mock token and decider
        Verifier implementation = new Verifier(address(endpoint));
        bytes memory initData = abi.encodeCall(
            Verifier.initialize,
            (
                address(mockToken),
                HUB_EID,
                address(this),
                address(mockDecider), // rootDecider
                address(mockDecider), // withdrawGlobalDecider
                address(mockDecider), // withdrawLocalDecider
                address(0x400), // singleWithdrawGlobalVerifier (not used in this test)
                address(0x500) // singleWithdrawLocalVerifier (not used in this test)
            )
        );
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        verifier = Verifier(address(proxy));
        verifier.setPeer(HUB_EID, _toBytes32(address(this)));

        attacker = new ReentrancyAttacker(verifier);
    }

    function testTeleportRevertsOnReentrancy() public {
        // Setup: store a global root so the teleport can proceed
        Origin memory origin = Origin({srcEid: HUB_EID, sender: _toBytes32(address(this)), nonce: 1});
        uint256 transferRoot = 12345;
        uint64 rootHint = 1;
        bytes memory payload = abi.encode(transferRoot, rootHint);
        vm.prank(address(endpoint));
        verifier.lzReceive(origin, bytes32(uint256(1)), payload, address(0), bytes(""));

        // Create valid GeneralRecipient for this chain
        GeneralRecipientLib.GeneralRecipient memory gr = GeneralRecipientLib.GeneralRecipient({
            chainId: uint64(block.chainid), recipient: bytes32(uint256(uint160(address(attacker)))), tweak: bytes32(0)
        });
        uint256 recipientHash = GeneralRecipientLib.hash(gr);

        // Build proof array that will pass mock verification
        uint256[34] memory proofArray;
        proofArray[1] = transferRoot; // transferRoot
        proofArray[2] = recipientHash; // recipient hash
        proofArray[3] = 0; // initialLastLeafIndex must be 0
        proofArray[4] = 0; // initialTotalValue must be 0
        proofArray[5] = transferRoot; // finalTransferRoot must match
        proofArray[6] = recipientHash; // finalRecipient must match
        proofArray[7] = 0; // lastLeafIndex
        proofArray[8] = 100; // totalValue to mint
        bytes memory proof = abi.encode(proofArray);

        // Setup callback: when mockToken.teleport is called, it will call attacker.attack
        // which attempts to call verifier.teleport again (reentrancy)
        bytes memory attackCalldata = abi.encodeCall(ReentrancyAttacker.attack, (true, rootHint, gr, proof));
        mockToken.setCallback(address(attacker), attackCalldata);

        // Execute teleport - this should succeed but the reentrant call should fail
        verifier.teleport(true, rootHint, gr, proof);

        // Verify the reentrant attack was attempted but failed
        assertTrue(attacker.attackAttempted(), "Attack should have been attempted");
        assertFalse(attacker.attackSucceeded(), "Reentrant attack should have failed");
    }

    function _toBytes32(address addr) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(addr)));
    }
}
