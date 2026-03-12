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
import {IWithdrawDecider} from "../src/interfaces/IWithdrawDecider.sol";
import {IRootDecider} from "../src/interfaces/IRootDecider.sol";
import {IWithdrawVerifier} from "../src/interfaces/IVerifier.sol";
import {IERC1271} from "@openzeppelin/contracts/interfaces/IERC1271.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";

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
                SINGLE_WITHDRAW_LOCAL_VERIFIER,
                abi.encode("Verifier", "1")
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
                SINGLE_WITHDRAW_LOCAL_VERIFIER,
                abi.encode("Verifier", "1")
            )
        );

        vm.expectRevert(Verifier.ZeroAddress.selector);
        new ERC1967Proxy(address(impl), initData);
    }

    function testInitializeRevertsOnZeroDelegate() public {
        Verifier impl = new Verifier(address(endpoint));
        bytes memory initData = abi.encodeCall(
            Verifier.initialize,
            (
                TOKEN,
                HUB_EID,
                address(0), // zero delegate
                ROOT_DECIDER,
                WITHDRAW_GLOBAL_DECIDER,
                WITHDRAW_LOCAL_DECIDER,
                SINGLE_WITHDRAW_GLOBAL_VERIFIER,
                SINGLE_WITHDRAW_LOCAL_VERIFIER,
                abi.encode("Verifier", "1")
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

    function testDeactivateEmergencyUnpauses() public {
        verifier.activateEmergency();
        assertTrue(verifier.paused(), "verifier should be paused");

        verifier.deactivateEmergency();
        assertFalse(verifier.paused(), "verifier should be unpaused");
    }

    function testDeactivateEmergencyOnlyOwner() public {
        verifier.activateEmergency();

        address nonOwner = address(0xBEEF);
        vm.prank(nonOwner);
        vm.expectRevert(abi.encodeWithSelector(OwnableUpgradeable.OwnableUnauthorizedAccount.selector, nonOwner));
        verifier.deactivateEmergency();
    }

    function testSetVerifiersOnlyOwner() public {
        address nonOwner = address(0xBEEF);
        vm.prank(nonOwner);
        vm.expectRevert(abi.encodeWithSelector(OwnableUpgradeable.OwnableUnauthorizedAccount.selector, nonOwner));
        verifier.setVerifiers(
            ROOT_DECIDER,
            WITHDRAW_GLOBAL_DECIDER,
            WITHDRAW_LOCAL_DECIDER,
            SINGLE_WITHDRAW_GLOBAL_VERIFIER,
            SINGLE_WITHDRAW_LOCAL_VERIFIER
        );
    }

    function testRelayTransferRootRevertsWhenPaused() public {
        verifier.activateEmergency();

        vm.deal(address(this), FEE_PER_MESSAGE);
        vm.expectRevert(PausableUpgradeable.EnforcedPause.selector);
        verifier.relayTransferRoot{value: FEE_PER_MESSAGE}(bytes(""));
    }

    function testRelayTransferRootRevertsOnInsufficientFee() public {
        uint256 insufficientFee = FEE_PER_MESSAGE - 1;
        vm.deal(address(this), insufficientFee);

        vm.expectRevert(
            abi.encodeWithSelector(Verifier.InsufficientMsgValue.selector, FEE_PER_MESSAGE, insufficientFee)
        );
        verifier.relayTransferRoot{value: insufficientFee}(bytes(""));
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

    function testLzReceiveIgnoresDuplicateRoot() public {
        Origin memory origin = Origin({srcEid: HUB_EID, sender: _toBytes32(address(this)), nonce: 1});
        uint64 aggSeq = 5;

        // First receive
        bytes memory payload1 = abi.encode(uint256(777), aggSeq);
        vm.prank(address(endpoint));
        verifier.lzReceive(origin, bytes32(uint256(1)), payload1, address(0), bytes(""));
        assertEq(verifier.globalTransferRoots(aggSeq), 777, "first root should be stored");

        // Second receive with different root - should be ignored
        bytes memory payload2 = abi.encode(uint256(999), aggSeq);
        vm.prank(address(endpoint));
        verifier.lzReceive(origin, bytes32(uint256(2)), payload2, address(0), bytes(""));
        assertEq(verifier.globalTransferRoots(aggSeq), 777, "root should not be overwritten");
    }

    function testLzReceiveRejectsInvalidSource() public {
        // Set peer for invalid EID so OApp's NoPeer check passes,
        // then InvalidHubSource should be triggered
        uint32 invalidEid = 999;
        verifier.setPeer(invalidEid, _toBytes32(address(this)));

        Origin memory origin = Origin({srcEid: invalidEid, sender: _toBytes32(address(this)), nonce: 1});
        bytes memory payload = abi.encode(uint256(777), uint64(5));

        vm.prank(address(endpoint));
        vm.expectRevert(abi.encodeWithSelector(Verifier.InvalidHubSource.selector, invalidEid));
        verifier.lzReceive(origin, bytes32(uint256(1)), payload, address(0), bytes(""));
    }

    function testLzReceiveUpdatesLatestAggSeq() public {
        Origin memory origin = Origin({srcEid: HUB_EID, sender: _toBytes32(address(this)), nonce: 1});

        // Receive seq 5
        vm.prank(address(endpoint));
        verifier.lzReceive(origin, bytes32(uint256(1)), abi.encode(uint256(100), uint64(5)), address(0), bytes(""));
        assertEq(verifier.latestAggSeq(), 5, "latestAggSeq should be 5");

        // Receive seq 3 (lower) - latestAggSeq should not decrease
        vm.prank(address(endpoint));
        verifier.lzReceive(origin, bytes32(uint256(2)), abi.encode(uint256(200), uint64(3)), address(0), bytes(""));
        assertEq(verifier.latestAggSeq(), 5, "latestAggSeq should still be 5");

        // Receive seq 10 (higher) - latestAggSeq should update
        vm.prank(address(endpoint));
        verifier.lzReceive(origin, bytes32(uint256(3)), abi.encode(uint256(300), uint64(10)), address(0), bytes(""));
        assertEq(verifier.latestAggSeq(), 10, "latestAggSeq should be 10");
    }

    function testIsUpToDateInitiallyTrue() public view {
        // Initially, latestProvedIndex = 0 and latestRelayedIndex = 0
        assertTrue(verifier.isUpToDate(), "should be up to date initially");
    }

    function testIsUpToDateAfterRelay() public {
        // Relay the initial root
        vm.deal(address(this), FEE_PER_MESSAGE);
        verifier.relayTransferRoot{value: FEE_PER_MESSAGE}(bytes(""));

        assertTrue(verifier.isUpToDate(), "should be up to date after relay");
        assertEq(verifier.latestRelayedIndex(), 0, "latestRelayedIndex should be 0");
    }

    function testRelayTransferRootRefundsExcessMsgValueToRefundAddress() public {
        address refundAddress = address(0xCAFE);
        uint256 excessiveFee = FEE_PER_MESSAGE + 1;
        vm.deal(address(this), excessiveFee);
        vm.deal(refundAddress, 0);

        uint256 refundBefore = refundAddress.balance;
        MessagingReceipt memory receipt = verifier.relayTransferRoot{value: excessiveFee}(bytes(""), refundAddress);
        assertEq(receipt.fee.nativeFee, FEE_PER_MESSAGE, "native fee mismatch");
        assertEq(refundAddress.balance, refundBefore + 1, "refund not received");
    }

    function testRelayTransferRootRevertsOnZeroRefundAddress() public {
        vm.deal(address(this), FEE_PER_MESSAGE);
        vm.expectRevert(Verifier.ZeroRefundAddress.selector);
        verifier.relayTransferRoot{value: FEE_PER_MESSAGE}(bytes(""), address(0));
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
                SINGLE_WITHDRAW_LOCAL_VERIFIER,
                abi.encode("Verifier", "1")
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

    // -----------------------------------------------------------------------
    // Allowed Prover Tests
    // -----------------------------------------------------------------------

    function testSetAllowedProver() public {
        address prover = address(0xBEEF);
        assertFalse(verifier.isAllowedProver(prover), "prover should not be allowed initially");
        verifier.setAllowedProver(prover);
        assertTrue(verifier.isAllowedProver(prover), "prover should be allowed after set");
    }

    function testSetAllowedProverEmitsEvent() public {
        address prover = address(0xBEEF);
        vm.expectEmit(true, false, false, false, address(verifier));
        emit Verifier.ProverAllowed(prover);
        verifier.setAllowedProver(prover);
    }

    function testSetAllowedProverOnlyOwner() public {
        address nonOwner = address(0xBEEF);
        vm.prank(nonOwner);
        vm.expectRevert(abi.encodeWithSelector(OwnableUpgradeable.OwnableUnauthorizedAccount.selector, nonOwner));
        verifier.setAllowedProver(address(0xCAFE));
    }

    function testSetAllowedProverRevertsOnZeroAddress() public {
        vm.expectRevert(Verifier.ZeroAddress.selector);
        verifier.setAllowedProver(address(0));
    }

    function testRemoveAllowedProver() public {
        address prover = address(0xBEEF);
        verifier.setAllowedProver(prover);
        assertTrue(verifier.isAllowedProver(prover), "prover should be allowed");
        verifier.removeAllowedProver(prover);
        assertFalse(verifier.isAllowedProver(prover), "prover should not be allowed after removal");
    }

    function testRemoveAllowedProverEmitsEvent() public {
        address prover = address(0xBEEF);
        verifier.setAllowedProver(prover);
        vm.expectEmit(true, false, false, false, address(verifier));
        emit Verifier.ProverRemoved(prover);
        verifier.removeAllowedProver(prover);
    }

    function testRemoveAllowedProverOnlyOwner() public {
        address nonOwner = address(0xBEEF);
        vm.prank(nonOwner);
        vm.expectRevert(abi.encodeWithSelector(OwnableUpgradeable.OwnableUnauthorizedAccount.selector, nonOwner));
        verifier.removeAllowedProver(address(0xCAFE));
    }

    function testRemoveAllowedProverRevertsOnZeroAddress() public {
        vm.expectRevert(Verifier.ZeroAddress.selector);
        verifier.removeAllowedProver(address(0));
    }

    function testIsAllowedProverDefaultFalse() public view {
        assertFalse(verifier.isAllowedProver(address(0xBEEF)), "should be false by default");
        assertFalse(verifier.isAllowedProver(address(this)), "should be false by default for test contract");
    }

    function testProveTransferRootRevertsWhenCallerNotAllowed() public {
        address nonProver = address(0xBEEF);
        uint256[32] memory proof;
        vm.prank(nonProver);
        vm.expectRevert(abi.encodeWithSelector(Verifier.UnauthorizedProver.selector, nonProver));
        verifier.proveTransferRoot(abi.encode(proof));
    }

    function testVerifierUpgradePreservesAllowedProvers() public {
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
                SINGLE_WITHDRAW_LOCAL_VERIFIER,
                abi.encode("Verifier", "1")
            )
        );
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        Verifier proxiedVerifier = Verifier(address(proxy));

        address prover = address(0xBEEF);
        proxiedVerifier.setAllowedProver(prover);
        assertTrue(proxiedVerifier.isAllowedProver(prover), "prover should be allowed before upgrade");

        VerifierUpgradeMock newImplementation = new VerifierUpgradeMock(address(endpoint));
        proxiedVerifier.upgradeToAndCall(address(newImplementation), bytes(""));

        assertTrue(proxiedVerifier.isAllowedProver(prover), "prover should be allowed after upgrade");
        assertFalse(proxiedVerifier.isAllowedProver(address(0xCAFE)), "non-prover should still be false after upgrade");
    }

    function testVerifierUpgradeRevertsOnEndpointMismatch() public {
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
                SINGLE_WITHDRAW_LOCAL_VERIFIER,
                abi.encode("Verifier", "1")
            )
        );
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        Verifier proxiedVerifier = Verifier(address(proxy));

        EndpointV2 otherEndpoint = endpointSetup.endpointList[1];
        VerifierUpgradeMock newImplementation = new VerifierUpgradeMock(address(otherEndpoint));
        vm.expectRevert(
            abi.encodeWithSelector(Verifier.EndpointMismatch.selector, address(endpoint), address(otherEndpoint))
        );
        proxiedVerifier.upgradeToAndCall(address(newImplementation), bytes(""));
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
                SINGLE_WITHDRAW_LOCAL_VERIFIER,
                abi.encode("Verifier", "1")
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
    uint256 public index;
    uint256 public hashChain;

    function setCallback(address target, bytes calldata data) external {
        callbackTarget = target;
        callbackData = data;
    }

    function setIndexAndHashChain(uint256 index_, uint256 hashChain_) external {
        index = index_;
        hashChain = hashChain_;
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
contract MockWithdrawDecider is IRootDecider, IWithdrawDecider {
    function verifyOpaqueNovaProof(uint256[32] calldata) external pure returns (bool) {
        return true;
    }

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

    event TransferRootProved(uint64 indexed index, uint256 root);
    event EmergencyTriggered(uint64 indexed index, uint256 root1, uint256 root2);

    function setUp() public override {
        super.setUp();
        setUpEndpoints(2, LibraryType.SimpleMessageLib);
        endpoint = endpointSetup.endpointList[0];

        mockToken = new MockZERC20WithCallback();
        mockDecider = new MockWithdrawDecider();
        mockToken.setIndexAndHashChain(42, 123);

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
                address(0x500), // singleWithdrawLocalVerifier (not used in this test)
                abi.encode("Verifier", "1")
            )
        );
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        verifier = Verifier(address(proxy));
        verifier.setPeer(HUB_EID, _toBytes32(address(this)));
        verifier.setAllowedProver(address(this));

        attacker = new ReentrancyAttacker(verifier);
    }

    function testReserveHashChainWorksWhenPaused() public {
        verifier.activateEmergency();
        (uint64 index_, uint256 hashChain_) = verifier.reserveHashChain();
        assertEq(index_, 42, "reserved index");
        assertEq(hashChain_, 123, "reserved hashChain");
        assertEq(verifier.latestReservedIndex(), 42, "latestReservedIndex");
        assertEq(verifier.reservedHashChains(42), 123, "reservedHashChains stored");
    }

    function testReserveHashChainRevertsWhenHashChainIsZero() public {
        mockToken.setIndexAndHashChain(7, 0);
        vm.expectRevert(abi.encodeWithSelector(Verifier.ZeroHashChain.selector, uint64(7)));
        verifier.reserveHashChain();
    }

    function testProveTransferRootRevertsWhenHashChainNotReserved() public {
        uint256 oldRoot = verifier.provedTransferRoots(0);
        uint64 oldIndex = 0;
        uint64 newIndex = 42;
        uint256 newHashChain = 123;
        uint256 newRoot = 777;
        uint256[32] memory proof;
        proof[1] = oldIndex;
        proof[3] = oldRoot;
        proof[4] = newIndex;
        proof[5] = newHashChain;
        proof[6] = newRoot;
        vm.expectRevert(abi.encodeWithSelector(Verifier.ReserveHashChainNotFound.selector, newIndex));
        verifier.proveTransferRoot(abi.encode(proof));
    }

    function testProveTransferRootRevertsOnHashChainMismatch() public {
        verifier.reserveHashChain();

        uint256 oldRoot = verifier.provedTransferRoots(0);
        uint64 oldIndex = 0;
        uint64 newIndex = 42;
        uint256 reservedHashChain = 123;
        uint256 providedHashChain = 999;
        uint256 newRoot = 777;
        uint256[32] memory proof;
        proof[1] = oldIndex;
        proof[3] = oldRoot;
        proof[4] = newIndex;
        proof[5] = providedHashChain;
        proof[6] = newRoot;
        vm.expectRevert(
            abi.encodeWithSelector(
                Verifier.NewHashChainMismatch.selector, newIndex, reservedHashChain, providedHashChain
            )
        );
        verifier.proveTransferRoot(abi.encode(proof));
    }

    function testProveTransferRootStoresNewRootAndUpdatesLatestProvedIndex() public {
        verifier.reserveHashChain();

        uint256 oldRoot = verifier.provedTransferRoots(0);
        uint64 oldIndex = 0;
        uint64 newIndex = 42;
        uint256 newHashChain = 123;
        uint256 newRoot = 777;
        uint256[32] memory proof;
        proof[1] = oldIndex;
        proof[3] = oldRoot;
        proof[4] = newIndex;
        proof[5] = newHashChain;
        proof[6] = newRoot;

        vm.expectEmit(true, true, false, true, address(verifier));
        emit TransferRootProved(newIndex, newRoot);
        verifier.proveTransferRoot(abi.encode(proof));

        assertEq(verifier.provedTransferRoots(newIndex), newRoot, "proved root stored");
        assertEq(verifier.latestProvedIndex(), newIndex, "latestProvedIndex updated");
    }

    function testProveTransferRootPausesOnConflictingRoot() public {
        verifier.reserveHashChain();

        uint256 oldRoot = verifier.provedTransferRoots(0);
        uint64 oldIndex = 0;
        uint64 newIndex = 42;
        uint256 newHashChain = 123;
        uint256 firstRoot = 777;
        uint256 secondRoot = 888;

        uint256[32] memory proof;
        proof[1] = oldIndex;
        proof[3] = oldRoot;
        proof[4] = newIndex;
        proof[5] = newHashChain;
        proof[6] = firstRoot;
        verifier.proveTransferRoot(abi.encode(proof));
        assertFalse(verifier.paused(), "should not pause on first proof");

        proof[6] = secondRoot;
        vm.expectEmit(true, false, false, true, address(verifier));
        emit EmergencyTriggered(newIndex, firstRoot, secondRoot);
        verifier.proveTransferRoot(abi.encode(proof));

        assertTrue(verifier.paused(), "should pause on conflict");
        assertEq(verifier.provedTransferRoots(newIndex), firstRoot, "conflicting proof should not overwrite root");
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

// ============================================================================
// Relayer Fee Tests
// ============================================================================

/// @dev Mock zERC20 that tracks teleport calls
contract MockZERC20ForRelayerFee {
    uint256 public index;
    uint256 public hashChain;

    struct TeleportCall {
        address to;
        uint256 value;
    }

    TeleportCall[] public teleportCalls;

    function setIndexAndHashChain(uint256 index_, uint256 hashChain_) external {
        index = index_;
        hashChain = hashChain_;
    }

    function teleport(address to, uint256 value) external {
        teleportCalls.push(TeleportCall({to: to, value: value}));
    }

    function teleportCallCount() external view returns (uint256) {
        return teleportCalls.length;
    }
}

/// @dev Mock Groth16 verifier that always returns true
contract MockSingleWithdrawVerifier is IWithdrawVerifier {
    function verifyProof(uint256[2] calldata, uint256[2][2] calldata, uint256[2] calldata, uint256[3] calldata)
        external
        pure
        returns (bool)
    {
        return true;
    }
}

/// @dev Mock ERC-1271 contract wallet
contract MockERC1271Wallet is IERC1271 {
    address public signer;

    constructor(address signer_) {
        signer = signer_;
    }

    function isValidSignature(bytes32 hash, bytes calldata signature) external view returns (bytes4) {
        (address recovered,,) = ECDSA.tryRecover(hash, signature);
        if (recovered == signer) {
            return IERC1271.isValidSignature.selector;
        }
        return bytes4(0xffffffff);
    }
}

contract VerifierRelayerFeeTest is TestHelperOz5 {
    Verifier internal verifier;
    EndpointV2 internal endpoint;
    MockZERC20ForRelayerFee internal mockToken;
    MockWithdrawDecider internal mockDecider;
    MockSingleWithdrawVerifier internal mockSingleVerifier;

    uint32 internal constant LOCAL_EID = 1;
    uint32 internal constant HUB_EID = 2;

    uint256 internal constant SIGNER_PK = 0xA11CE;
    address internal signerAddr;

    bytes32 internal constant RELAYER_FEE_TYPEHASH =
        keccak256("RelayerFeeAuthorization(uint256 recipientHash,uint256 totalValue,uint256 maxFee,uint64 deadline)");

    uint256 internal constant TRANSFER_ROOT = 12345;
    uint64 internal constant ROOT_HINT = 1;

    function setUp() public override {
        super.setUp();
        setUpEndpoints(2, LibraryType.SimpleMessageLib);
        endpoint = endpointSetup.endpointList[0];

        signerAddr = vm.addr(SIGNER_PK);

        mockToken = new MockZERC20ForRelayerFee();
        mockDecider = new MockWithdrawDecider();
        mockSingleVerifier = new MockSingleWithdrawVerifier();
        mockToken.setIndexAndHashChain(42, 123);

        Verifier implementation = new Verifier(address(endpoint));
        bytes memory initData = abi.encodeCall(
            Verifier.initialize,
            (
                address(mockToken),
                HUB_EID,
                address(this),
                address(mockDecider),
                address(mockDecider),
                address(mockDecider),
                address(mockSingleVerifier),
                address(mockSingleVerifier),
                abi.encode("Verifier", "1")
            )
        );
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        verifier = Verifier(address(proxy));
        verifier.setPeer(HUB_EID, _toBytes32(address(this)));

        // Store a global root for testing
        Origin memory origin = Origin({srcEid: HUB_EID, sender: _toBytes32(address(this)), nonce: 1});
        bytes memory payload = abi.encode(TRANSFER_ROOT, ROOT_HINT);
        vm.prank(address(endpoint));
        verifier.lzReceive(origin, bytes32(uint256(1)), payload, address(0), bytes(""));
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    function _buildGr(address recipient) internal view returns (GeneralRecipientLib.GeneralRecipient memory) {
        return GeneralRecipientLib.GeneralRecipient({
            chainId: uint64(block.chainid), recipient: bytes32(uint256(uint160(recipient))), tweak: bytes32(0)
        });
    }

    function _buildNovaProof(uint256 transferRoot, uint256 recipientHash, uint256 totalValue)
        internal
        pure
        returns (bytes memory)
    {
        uint256[34] memory proofArray;
        proofArray[1] = transferRoot;
        proofArray[2] = recipientHash;
        proofArray[3] = 0; // initialLastLeafIndex
        proofArray[4] = 0; // initialTotalValue
        proofArray[5] = transferRoot; // finalTransferRoot
        proofArray[6] = recipientHash; // finalRecipient
        proofArray[7] = 0; // lastLeafIndex
        proofArray[8] = totalValue;
        return abi.encode(proofArray);
    }

    function _buildSingleProof(uint256 transferRoot, uint256 recipientHash, uint256 totalValue)
        internal
        pure
        returns (bytes memory)
    {
        uint256[2] memory pA;
        uint256[2][2] memory pB;
        uint256[2] memory pC;
        uint256[3] memory pubSignals = [transferRoot, recipientHash, totalValue];
        return abi.encode(pA, pB, pC, pubSignals);
    }

    function _signRelayerFeeAuth(
        uint256 privateKey,
        uint256 recipientHash,
        uint256 totalValue,
        uint256 maxFee,
        uint64 deadline
    ) internal view returns (bytes memory) {
        bytes32 structHash = keccak256(abi.encode(RELAYER_FEE_TYPEHASH, recipientHash, totalValue, maxFee, deadline));
        // Use the proper EIP-712 hash
        bytes32 digest = _hashTypedDataV4(structHash);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(privateKey, digest);
        return abi.encodePacked(r, s, v);
    }

    function _hashTypedDataV4(bytes32 structHash) internal view returns (bytes32) {
        // Reconstruct the EIP-712 hash using domain separator
        (, string memory name_, string memory version_, uint256 chainId_, address verifyingContract_,,) =
            verifier.eip712Domain();
        bytes32 domainSeparator = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256(bytes(name_)),
                keccak256(bytes(version_)),
                chainId_,
                verifyingContract_
            )
        );
        return MessageHashUtils.toTypedDataHash(domainSeparator, structHash);
    }

    function _buildFeeAuth(uint256 relayerFee, uint256 maxFee, uint64 deadline, bytes memory signature)
        internal
        pure
        returns (Verifier.RelayerFeeAuthorization memory)
    {
        return Verifier.RelayerFeeAuthorization({
            relayerFee: relayerFee, maxFee: maxFee, deadline: deadline, signature: signature
        });
    }

    // -----------------------------------------------------------------------
    // initializeV2 tests
    // -----------------------------------------------------------------------

    function testInitializeV2SetsEIP712() public view {
        (, string memory name_,,,,,) = verifier.eip712Domain();
        assertEq(name_, "Verifier", "EIP-712 name mismatch");
    }

    function testInitializeV2CannotBeCalledTwice() public {
        // initialize sets version to 1, so initializeV2 (reinitializer(2)) succeeds once
        verifier.initializeV2("Verifier", "1");
        // Second call should revert because version is already 2
        vm.expectRevert();
        verifier.initializeV2("Verifier", "1");
    }

    function testInitializeV2OnlyOwner() public {
        address nonOwner = address(0xBEEF);
        vm.prank(nonOwner);
        vm.expectRevert(abi.encodeWithSelector(OwnableUpgradeable.OwnableUnauthorizedAccount.selector, nonOwner));
        verifier.initializeV2("Verifier", "1");
    }

    // -----------------------------------------------------------------------
    // Happy path: Nova teleport with relayer fee
    // -----------------------------------------------------------------------

    function testTeleportWithRelayerFeeNova() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000;
        uint256 relayerFee = 50;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        address relayer = address(0xBE1A);
        vm.prank(relayer);
        verifier.teleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(relayerFee, maxFee, deadline, signature));

        // Check: recipient gets totalValue - relayerFee
        (address recipientTo, uint256 recipientValue) = mockToken.teleportCalls(0);
        assertEq(recipientTo, signerAddr, "recipient address mismatch");
        assertEq(recipientValue, totalValue - relayerFee, "recipient value mismatch");

        // Check: relayer gets relayerFee
        (address relayerTo, uint256 relayerValue) = mockToken.teleportCalls(1);
        assertEq(relayerTo, relayer, "relayer address mismatch");
        assertEq(relayerValue, relayerFee, "relayer value mismatch");

        // Check totalTeleported updated
        assertEq(verifier.totalTeleported(recipientHash), totalValue, "totalTeleported mismatch");
    }

    // -----------------------------------------------------------------------
    // Happy path: Groth16 singleTeleport with relayer fee
    // -----------------------------------------------------------------------

    function testSingleTeleportWithRelayerFee() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 500;
        uint256 relayerFee = 25;
        uint256 maxFee = 50;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildSingleProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        address relayer = address(0xBE1A);
        vm.prank(relayer);
        verifier.singleTeleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(relayerFee, maxFee, deadline, signature));

        (address recipientTo, uint256 recipientValue) = mockToken.teleportCalls(0);
        assertEq(recipientTo, signerAddr, "recipient address mismatch");
        assertEq(recipientValue, totalValue - relayerFee, "recipient value mismatch");

        (address relayerTo, uint256 relayerValue) = mockToken.teleportCalls(1);
        assertEq(relayerTo, relayer, "relayer address mismatch");
        assertEq(relayerValue, relayerFee, "relayer value mismatch");
    }

    // -----------------------------------------------------------------------
    // Zero relayer fee (no extra teleport call)
    // -----------------------------------------------------------------------

    function testTeleportWithZeroRelayerFee() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000;
        uint256 relayerFee = 0;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        address relayer = address(0xBE1A);
        vm.prank(relayer);
        verifier.teleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(relayerFee, maxFee, deadline, signature));

        // Only 1 teleport call (to recipient), no relayer call
        assertEq(mockToken.teleportCallCount(), 1, "should be exactly 1 teleport call");
        (address recipientTo, uint256 recipientValue) = mockToken.teleportCalls(0);
        assertEq(recipientTo, signerAddr, "recipient address mismatch");
        assertEq(recipientValue, totalValue, "recipient value mismatch");
    }

    // -----------------------------------------------------------------------
    // Signature validation: invalid signature
    // -----------------------------------------------------------------------

    function testRevertsOnInvalidSignature() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        // Sign with wrong key
        uint256 wrongPk = 0xDEAD;
        bytes memory badSignature = _signRelayerFeeAuth(wrongPk, recipientHash, totalValue, maxFee, deadline);

        vm.expectRevert(Verifier.InvalidRelayerFeeSignature.selector);
        verifier.teleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(50, maxFee, deadline, badSignature));
    }

    // -----------------------------------------------------------------------
    // Signature validation: expired deadline
    // -----------------------------------------------------------------------

    function testRevertsOnExpiredDeadline() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp - 1); // already expired

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        vm.expectRevert(
            abi.encodeWithSelector(Verifier.RelayerFeeAuthorizationExpired.selector, deadline, block.timestamp)
        );
        verifier.teleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(50, maxFee, deadline, signature));
    }

    // -----------------------------------------------------------------------
    // Fee limits: relayerFee > maxFee
    // -----------------------------------------------------------------------

    function testRevertsWhenRelayerFeeExceedsMaxFee() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        uint256 relayerFee = 101; // exceeds maxFee
        vm.expectRevert(abi.encodeWithSelector(Verifier.RelayerFeeExceedsMax.selector, relayerFee, maxFee));
        verifier.teleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(relayerFee, maxFee, deadline, signature));
    }

    // -----------------------------------------------------------------------
    // Fee limits: relayerFee > diff
    // -----------------------------------------------------------------------

    function testRevertsWhenRelayerFeeExceedsDiff() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000;
        uint256 maxFee = 1500; // maxFee > totalValue to isolate diff check
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        uint256 relayerFee = 1001; // exceeds diff (which is totalValue = 1000)
        vm.expectRevert(abi.encodeWithSelector(Verifier.RelayerFeeExceedsDiff.selector, relayerFee, totalValue));
        verifier.teleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(relayerFee, maxFee, deadline, signature));
    }

    // -----------------------------------------------------------------------
    // Natural nonce: same totalValue replay is impossible
    // -----------------------------------------------------------------------

    function testReplayWithSameTotalValueReverts() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        // First teleport succeeds
        verifier.teleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(50, maxFee, deadline, signature));

        // Second teleport with same totalValue reverts (NothingToWithdraw)
        vm.expectRevert(abi.encodeWithSelector(Verifier.NothingToWithdraw.selector, totalValue, totalValue));
        verifier.teleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(50, maxFee, deadline, signature));
    }

    // -----------------------------------------------------------------------
    // Backward compatibility: legacy teleport still works
    // -----------------------------------------------------------------------

    function testLegacyTeleportStillWorks() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000;

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);

        verifier.teleport(true, ROOT_HINT, gr, proof);

        assertEq(mockToken.teleportCallCount(), 1, "should be exactly 1 teleport call");
        (address recipientTo, uint256 recipientValue) = mockToken.teleportCalls(0);
        assertEq(recipientTo, signerAddr, "recipient address mismatch");
        assertEq(recipientValue, totalValue, "recipient value mismatch");
        assertEq(verifier.totalTeleported(recipientHash), totalValue, "totalTeleported mismatch");
    }

    function testLegacySingleTeleportStillWorks() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 500;

        bytes memory proof = _buildSingleProof(TRANSFER_ROOT, recipientHash, totalValue);

        verifier.singleTeleport(true, ROOT_HINT, gr, proof);

        assertEq(mockToken.teleportCallCount(), 1, "should be exactly 1 teleport call");
        (address recipientTo, uint256 recipientValue) = mockToken.teleportCalls(0);
        assertEq(recipientTo, signerAddr, "recipient address mismatch");
        assertEq(recipientValue, totalValue, "recipient value mismatch");
    }

    // -----------------------------------------------------------------------
    // ERC-1271: contract wallet signature verification
    // -----------------------------------------------------------------------

    function testERC1271ContractWalletSignature() public {
        // Deploy mock wallet that delegates to signerAddr
        MockERC1271Wallet wallet = new MockERC1271Wallet(signerAddr);

        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(address(wallet));
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        // Sign with the EOA key that the wallet recognizes
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        address relayer = address(0xBE1A);
        vm.prank(relayer);
        verifier.teleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(50, maxFee, deadline, signature));

        (address recipientTo, uint256 recipientValue) = mockToken.teleportCalls(0);
        assertEq(recipientTo, address(wallet), "recipient address mismatch");
        assertEq(recipientValue, totalValue - 50, "recipient value mismatch");
    }

    // -----------------------------------------------------------------------
    // Event emission
    // -----------------------------------------------------------------------

    function testTeleportWithRelayerFeeEmitsEvent() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000;
        uint256 relayerFee = 50;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        address relayer = address(0xBE1A);
        vm.prank(relayer);

        vm.recordLogs();
        verifier.teleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(relayerFee, maxFee, deadline, signature));

        Vm.Log[] memory logs = vm.getRecordedLogs();
        bytes32 eventSig = keccak256(
            "TeleportWithRelayerFee(address,address,uint256,uint256,bool,uint64,uint256,(uint64,bytes32,bytes32))"
        );
        bool found;
        for (uint256 i = 0; i < logs.length; ++i) {
            if (logs[i].topics.length > 0 && logs[i].topics[0] == eventSig) {
                assertEq(address(uint160(uint256(logs[i].topics[1]))), signerAddr, "indexed to mismatch");
                assertEq(address(uint160(uint256(logs[i].topics[2]))), relayer, "indexed relayer mismatch");
                found = true;
            }
        }
        assertTrue(found, "TeleportWithRelayerFee not emitted");
    }

    // -----------------------------------------------------------------------
    // Incremental totalValue works (second teleport with higher totalValue)
    // -----------------------------------------------------------------------

    function testIncrementalTeleportWithRelayerFee() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);

        // First teleport: totalValue = 1000
        {
            uint64 deadline1 = uint64(block.timestamp + 1 hours);
            bytes memory proof1 = _buildNovaProof(TRANSFER_ROOT, recipientHash, 1000);
            bytes memory sig1 = _signRelayerFeeAuth(SIGNER_PK, recipientHash, 1000, 100, deadline1);
            verifier.teleport(true, ROOT_HINT, gr, proof1, _buildFeeAuth(50, 100, deadline1, sig1));
        }

        // Second teleport: totalValue = 2000 (diff = 1000)
        {
            uint64 deadline2 = uint64(block.timestamp + 2 hours);
            bytes memory proof2 = _buildNovaProof(TRANSFER_ROOT, recipientHash, 2000);
            bytes memory sig2 = _signRelayerFeeAuth(SIGNER_PK, recipientHash, 2000, 200, deadline2);

            address relayer = address(0xBE1A);
            vm.prank(relayer);
            verifier.teleport(true, ROOT_HINT, gr, proof2, _buildFeeAuth(75, 200, deadline2, sig2));
        }

        // Second teleport: recipient gets diff - relayerFee = 1000 - 75 = 925
        (address recipientTo, uint256 recipientValue) = mockToken.teleportCalls(2);
        assertEq(recipientTo, signerAddr, "recipient address mismatch");
        assertEq(recipientValue, 925, "recipient value mismatch");

        assertEq(verifier.totalTeleported(recipientHash), 2000, "totalTeleported mismatch");
    }

    // -----------------------------------------------------------------------
    // Paused contract reverts for relayer teleport
    // -----------------------------------------------------------------------

    function testRelayerTeleportRevertsWhenPaused() public {
        verifier.activateEmergency();

        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        vm.expectRevert(PausableUpgradeable.EnforcedPause.selector);
        verifier.teleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(50, maxFee, deadline, signature));
    }

    // -----------------------------------------------------------------------
    // singleTeleport error paths
    // -----------------------------------------------------------------------

    function testSingleTeleportRevertsOnInvalidSignature() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 500;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildSingleProof(TRANSFER_ROOT, recipientHash, totalValue);
        uint256 wrongPk = 0xDEAD;
        bytes memory badSignature = _signRelayerFeeAuth(wrongPk, recipientHash, totalValue, maxFee, deadline);

        vm.expectRevert(Verifier.InvalidRelayerFeeSignature.selector);
        verifier.singleTeleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(50, maxFee, deadline, badSignature));
    }

    function testSingleTeleportRevertsOnExpiredDeadline() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 500;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp - 1);

        bytes memory proof = _buildSingleProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        vm.expectRevert(
            abi.encodeWithSelector(Verifier.RelayerFeeAuthorizationExpired.selector, deadline, block.timestamp)
        );
        verifier.singleTeleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(50, maxFee, deadline, signature));
    }

    function testSingleTeleportRevertsWhenFeeExceedsMax() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 500;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildSingleProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        uint256 relayerFee = 101;
        vm.expectRevert(abi.encodeWithSelector(Verifier.RelayerFeeExceedsMax.selector, relayerFee, maxFee));
        verifier.singleTeleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(relayerFee, maxFee, deadline, signature));
    }

    function testSingleTeleportRevertsWhenFeeExceedsDiff() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 500;
        uint256 maxFee = 600;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildSingleProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        uint256 relayerFee = 501;
        vm.expectRevert(abi.encodeWithSelector(Verifier.RelayerFeeExceedsDiff.selector, relayerFee, totalValue));
        verifier.singleTeleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(relayerFee, maxFee, deadline, signature));
    }

    function testSingleTeleportReplayReverts() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 500;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildSingleProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        verifier.singleTeleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(50, maxFee, deadline, signature));

        vm.expectRevert(abi.encodeWithSelector(Verifier.NothingToWithdraw.selector, totalValue, totalValue));
        verifier.singleTeleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(50, maxFee, deadline, signature));
    }

    function testSingleTeleportRelayerFeeRevertsWhenPaused() public {
        verifier.activateEmergency();

        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 500;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildSingleProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        vm.expectRevert(PausableUpgradeable.EnforcedPause.selector);
        verifier.singleTeleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(50, maxFee, deadline, signature));
    }

    // -----------------------------------------------------------------------
    // Boundary: relayerFee == maxFee
    // -----------------------------------------------------------------------

    function testRelayerFeeEqualsMaxFeeSucceeds() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000;
        uint256 maxFee = 100;
        uint256 relayerFee = 100; // exactly maxFee
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        address relayer = address(0xBE1A);
        vm.prank(relayer);
        verifier.teleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(relayerFee, maxFee, deadline, signature));

        (address recipientTo, uint256 recipientValue) = mockToken.teleportCalls(0);
        assertEq(recipientTo, signerAddr, "recipient address mismatch");
        assertEq(recipientValue, totalValue - relayerFee, "recipient value mismatch");

        (address relayerTo, uint256 relayerValue) = mockToken.teleportCalls(1);
        assertEq(relayerTo, relayer, "relayer address mismatch");
        assertEq(relayerValue, relayerFee, "relayer value mismatch");
    }

    // -----------------------------------------------------------------------
    // Boundary: relayerFee == diff (recipient gets zero)
    // -----------------------------------------------------------------------

    function testRelayerFeeEqualsDiffRecipientGetsZero() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000;
        uint256 relayerFee = 1000; // entire diff goes to relayer
        uint256 maxFee = 1000;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        address relayer = address(0xBE1A);
        vm.prank(relayer);
        verifier.teleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(relayerFee, maxFee, deadline, signature));

        // recipient gets 0
        (address recipientTo, uint256 recipientValue) = mockToken.teleportCalls(0);
        assertEq(recipientTo, signerAddr, "recipient address mismatch");
        assertEq(recipientValue, 0, "recipient should get zero");

        // relayer gets full amount
        (address relayerTo, uint256 relayerValue) = mockToken.teleportCalls(1);
        assertEq(relayerTo, relayer, "relayer address mismatch");
        assertEq(relayerValue, totalValue, "relayer should get full amount");
    }

    // -----------------------------------------------------------------------
    // Local root (isGlobal=false) with relayer fee
    // -----------------------------------------------------------------------

    function testTeleportWithRelayerFeeLocalRoot() public {
        // Store a local proved root
        verifier.reserveHashChain();
        verifier.setAllowedProver(address(this));

        uint256 oldRoot = verifier.provedTransferRoots(0);
        uint64 newIndex = 42;
        uint256 newHashChain = 123;
        uint256 localRoot = 99999;
        uint256[32] memory rootProof;
        rootProof[1] = 0; // oldIndex
        rootProof[3] = oldRoot;
        rootProof[4] = newIndex;
        rootProof[5] = newHashChain;
        rootProof[6] = localRoot;
        verifier.proveTransferRoot(abi.encode(rootProof));

        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 800;
        uint256 relayerFee = 30;
        uint256 maxFee = 50;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildNovaProof(localRoot, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        address relayer = address(0xBE1A);
        vm.prank(relayer);
        // isGlobal = false, rootHint = newIndex
        verifier.teleport(false, newIndex, gr, proof, _buildFeeAuth(relayerFee, maxFee, deadline, signature));

        (address recipientTo, uint256 recipientValue) = mockToken.teleportCalls(0);
        assertEq(recipientTo, signerAddr, "recipient address mismatch");
        assertEq(recipientValue, totalValue - relayerFee, "recipient value mismatch");

        (address relayerTo, uint256 relayerValue) = mockToken.teleportCalls(1);
        assertEq(relayerTo, relayer, "relayer address mismatch");
        assertEq(relayerValue, relayerFee, "relayer value mismatch");
    }

    // -----------------------------------------------------------------------
    // Deadline boundary: deadline == block.timestamp should succeed
    // -----------------------------------------------------------------------

    function testDeadlineExactlyAtBlockTimestampSucceeds() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000;
        uint256 relayerFee = 50;
        uint256 maxFee = 100;
        uint64 deadline = uint64(block.timestamp); // exactly now

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);

        // Should succeed because Verifier checks block.timestamp <= deadline
        verifier.teleport(true, ROOT_HINT, gr, proof, _buildFeeAuth(relayerFee, maxFee, deadline, signature));

        assertEq(verifier.totalTeleported(recipientHash), totalValue, "totalTeleported mismatch");
    }

    function _toBytes32(address addr) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(addr)));
    }
}
