// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Vm} from "forge-std/Vm.sol";
import {Hub} from "../src/Hub.sol";
import {Origin} from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/ILayerZeroReceiver.sol";
import {OptionsBuilder} from "@layerzerolabs/oapp-evm/contracts/oapp/libs/OptionsBuilder.sol";
import {IOAppCore} from "@layerzerolabs/oapp-evm/contracts/oapp/interfaces/IOAppCore.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {
    TestHelperOz5,
    EndpointV2,
    SimpleMessageLibMock
} from "@layerzerolabs/test-devtools-evm-foundry/contracts/TestHelperOz5.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";

contract HubTest is TestHelperOz5 {
    using OptionsBuilder for bytes;

    Hub internal hub;
    EndpointV2 internal endpoint;
    SimpleMessageLibMock internal sendLib;

    uint32 internal constant LOCAL_EID = 1;
    uint32 internal constant REMOTE_EID_A = 2;
    uint32 internal constant REMOTE_EID_B = 3;
    uint256 internal constant FEE_PER_MESSAGE = 0.05 ether;

    address internal constant REMOTE_PEER_A = address(0xAA);
    address internal constant REMOTE_PEER_B = address(0xBB);

    bytes32 internal constant PACKET_SENT_SIG = keccak256("PacketSent(bytes,bytes,address)");

    function setUp() public override {
        super.setUp();
        setUpEndpoints(3, LibraryType.SimpleMessageLib);
        endpoint = endpointSetup.endpointList[0];
        sendLib = SimpleMessageLibMock(payable(endpointSetup.sendLibs[0]));
        sendLib.setMessagingFee(FEE_PER_MESSAGE, 0);

        hub = _deployInitializedHub();

        hub.setPeer(REMOTE_EID_A, _toBytes32(REMOTE_PEER_A));
        hub.setPeer(REMOTE_EID_B, _toBytes32(REMOTE_PEER_B));

        hub.registerToken(Hub.TokenInfo({chainId: 101, eid: REMOTE_EID_A, verifier: address(0x1), token: address(0x2)}));
        hub.registerToken(Hub.TokenInfo({chainId: 202, eid: REMOTE_EID_B, verifier: address(0x3), token: address(0x4)}));
    }

    function testConstructorRevertsOnZeroEndpoint() public {
        vm.expectRevert(IOAppCore.InvalidEndpointCall.selector);
        new Hub(address(0));
    }

    function testQuoteBroadcastAggregatesFees() public view {
        uint32[] memory targetEids = _targetEids();
        bytes memory options = _options();

        uint256 total = hub.quoteBroadcast(targetEids, options);
        assertEq(total, FEE_PER_MESSAGE * targetEids.length, "total native fee");
    }

    function testBroadcastRevertsWhenUnderfunded() public {
        uint32[] memory targetEids = _targetEids();
        bytes memory options = _options();

        uint256 total = hub.quoteBroadcast(targetEids, options);
        vm.deal(address(this), total);

        vm.expectRevert(abi.encodeWithSelector(Hub.NativeFeeMismatch.selector, total - 1, total));
        hub.broadcast{value: total - 1}(targetEids, options);
    }

    function testBroadcastRevertsWhenNoTargetsProvided() public {
        uint32[] memory targetEids = new uint32[](0);
        bytes memory options = _options();

        vm.expectRevert(Hub.EmptyTargetEids.selector);
        hub.broadcast(targetEids, options);
    }

    function testBroadcastRevertsWhenAggregationRootIsZero() public {
        Hub localHub = _deployInitializedZeroRootHub();
        uint32[] memory targetEids = _targetEids();
        bytes memory options = _options();

        vm.expectRevert(Hub.AggregationRootZero.selector);
        localHub.broadcast(targetEids, options);
    }

    function testQuoteBroadcastRevertsWhenNoTargetsProvided() public {
        uint32[] memory targetEids = new uint32[](0);
        bytes memory options = _options();

        vm.expectRevert(Hub.EmptyTargetEids.selector);
        hub.quoteBroadcast(targetEids, options);
    }

    function testBroadcastPaysFeesAndRefundsExcess() public {
        uint32[] memory targetEids = _targetEids();
        bytes memory options = _options();

        uint256 total = hub.quoteBroadcast(targetEids, options);
        uint256 deposit = total + 0.02 ether;

        vm.deal(address(this), deposit);
        uint256 balanceBefore = address(this).balance;

        vm.recordLogs();
        hub.broadcast{value: deposit}(targetEids, options);
        Vm.Log[] memory logs = vm.getRecordedLogs();

        uint256 packetCount;
        for (uint256 i = 0; i < logs.length; ++i) {
            if (logs[i].topics[0] == PACKET_SENT_SIG) {
                ++packetCount;
                (bytes memory encodedPacket, bytes memory emittedOptions, address sendLibrary) =
                    abi.decode(logs[i].data, (bytes, bytes, address));
                assertEq(sendLibrary, address(sendLib), "send library");
                assertEq(emittedOptions, options, "options forwarded");
                assertTrue(encodedPacket.length > 0, "packet encoded");
            }
        }
        assertEq(packetCount, targetEids.length, "packets sent");

        uint256 balanceAfter = address(this).balance;
        assertEq(balanceBefore - balanceAfter, total, "net cost");
        assertEq(hub.aggSeq(), 1, "agg sequence incremented");
    }

    function testAggSeqIncrementsWithEachBroadcast() public {
        uint32[] memory targetEids = _targetEids();
        bytes memory options = _options();
        uint256 total = hub.quoteBroadcast(targetEids, options);

        vm.deal(address(this), total * 2);
        hub.broadcast{value: total}(targetEids, options);
        assertEq(hub.aggSeq(), 1, "first broadcast increments aggSeq");

        hub.broadcast{value: total}(targetEids, options);
        assertEq(hub.aggSeq(), 2, "second broadcast increments aggSeq");
    }

    function testLogLzReceiveOptionZeroValue() public {
        bytes memory options = OptionsBuilder.newOptions();
        options = options.addExecutorLzReceiveOption(200_000, 0);

        emit log_named_bytes("lzReceiveOptionZeroValue", options);
        assertGt(options.length, 0, "options should not be empty");
    }

    function testRegisterTokenStoresMetadata() public {
        Hub localHub = _deployInitializedHub();
        Hub.TokenInfo memory info =
            Hub.TokenInfo({chainId: 505, eid: 55, verifier: address(0x55), token: address(0x99)});

        vm.expectEmit(true, true, true, true, address(localHub));
        emit Hub.TokenRegistered(info.eid, 0, info.chainId, info.token, info.verifier);
        localHub.registerToken(info);

        (uint64 chainId, uint32 eid, address verifier, address tokenAddr) = localHub.tokenInfos(0);
        assertEq(chainId, info.chainId, "chain id stored");
        assertEq(eid, info.eid, "eid stored");
        assertEq(verifier, info.verifier, "verifier stored");
        assertEq(tokenAddr, info.token, "token stored");
        assertEq(localHub.eidToPosition(info.eid), 1, "eid to position mapping");
        assertEq(localHub.transferRoots(0), 0, "initial transfer root zero");
        assertEq(localHub.transferTreeIndices(0), 0, "initial transfer tree index zero");
    }

    function testRegisterTokenValidationReverts() public {
        Hub localHub = _deployInitializedHub();

        vm.expectRevert(Hub.ZeroVerifier.selector);
        localHub.registerToken(Hub.TokenInfo({chainId: 1, eid: 10, verifier: address(0), token: address(0x1)}));

        vm.expectRevert(Hub.ZeroToken.selector);
        localHub.registerToken(Hub.TokenInfo({chainId: 1, eid: 11, verifier: address(0x1), token: address(0)}));

        vm.expectRevert(Hub.InvalidChainId.selector);
        localHub.registerToken(Hub.TokenInfo({chainId: 0, eid: 12, verifier: address(0x1), token: address(0x2)}));
    }

    function testRegisterTokenDuplicateEidReverts() public {
        Hub.TokenInfo memory duplicate =
            Hub.TokenInfo({chainId: 999, eid: REMOTE_EID_A, verifier: address(0x5), token: address(0x6)});

        vm.expectRevert(abi.encodeWithSelector(Hub.TokenAlreadyRegistered.selector, REMOTE_EID_A));
        hub.registerToken(duplicate);
    }

    function testUpdateTokenUpdatesStructAndEmits() public {
        Hub.TokenInfo memory updated =
            Hub.TokenInfo({chainId: 303, eid: REMOTE_EID_A, verifier: address(0xA), token: address(0xB)});

        vm.expectEmit(true, true, true, true, address(hub));
        emit Hub.TokenUpdated(updated.eid, 0, updated.chainId, updated.token, updated.verifier);
        hub.updateToken(updated);

        (uint64 chainId, uint32 eid, address verifier, address tokenAddr) = hub.tokenInfos(0);
        assertEq(chainId, updated.chainId, "chain id updated");
        assertEq(eid, updated.eid, "eid persisted");
        assertEq(verifier, updated.verifier, "verifier updated");
        assertEq(tokenAddr, updated.token, "token updated");
    }

    function testUpdateTokenMissingEntryReverts() public {
        Hub.TokenInfo memory missing =
            Hub.TokenInfo({chainId: 111, eid: 77, verifier: address(0xC), token: address(0xD)});

        vm.expectRevert(abi.encodeWithSelector(Hub.TokenNotRegistered.selector, missing.eid));
        hub.updateToken(missing);
    }

    function testQuoteBroadcastUnknownEidReverts() public {
        uint32[] memory eids = new uint32[](1);
        eids[0] = 444;
        bytes memory options = _options();

        vm.expectRevert(abi.encodeWithSelector(Hub.TokenNotRegistered.selector, eids[0]));
        hub.quoteBroadcast(eids, options);
    }

    function testGetTokenInfosReturnsSnapshot() public view {
        Hub.TokenInfo[] memory infos = hub.getTokenInfos();
        assertEq(infos.length, 2, "length");
        assertEq(infos[0].eid, REMOTE_EID_A, "first eid");
        assertEq(infos[0].chainId, 101, "first chain id");
        assertEq(infos[1].eid, REMOTE_EID_B, "second eid");
        assertEq(infos[1].verifier, address(0x3), "second verifier");
    }

    function testLzReceiveRevertsWhenUnregisteredEid() public {
        Origin memory origin = Origin({srcEid: 999, sender: _toBytes32(address(this)), nonce: 1});
        bytes memory payload = abi.encode(uint256(1), uint64(1));

        hub.setPeer(origin.srcEid, _toBytes32(address(this)));

        vm.prank(address(endpoint));
        vm.expectRevert(abi.encodeWithSelector(Hub.TokenNotRegistered.selector, origin.srcEid));
        hub.lzReceive(origin, bytes32(0), payload, address(0), bytes(""));
    }

    function testLzReceiveRevertsOnInvalidPayloadLength() public {
        Origin memory origin = Origin({srcEid: REMOTE_EID_A, sender: _toBytes32(address(this)), nonce: 1});
        bytes memory payload = hex"01";

        hub.setPeer(REMOTE_EID_A, _toBytes32(address(this)));

        vm.prank(address(endpoint));
        vm.expectRevert(abi.encodeWithSelector(Hub.InvalidPayloadLength.selector, payload.length));
        hub.lzReceive(origin, bytes32(0), payload, address(0), bytes(""));
    }

    function testLzReceiveUpdatesRoot() public {
        Hub localHub = _deployInitializedHub();
        Hub.TokenInfo memory info = Hub.TokenInfo({chainId: 909, eid: 77, verifier: address(0x7), token: address(0x8)});
        localHub.registerToken(info);

        Origin memory origin = Origin({srcEid: info.eid, sender: _toBytes32(address(this)), nonce: 1});
        uint256 newRoot = 777;
        uint64 treeIndex = 5;
        bytes memory payload = abi.encode(newRoot, treeIndex);

        localHub.setPeer(info.eid, _toBytes32(address(this)));

        vm.expectEmit(true, true, true, true, address(localHub));
        emit Hub.TransferRootUpdated(info.eid, 0, newRoot);

        vm.prank(address(endpoint));
        localHub.lzReceive(origin, bytes32(0), payload, address(0), bytes(""));

        assertEq(localHub.transferRoots(0), newRoot, "root stored");
        assertEq(localHub.transferTreeIndices(0), treeIndex, "tree index stored");
    }

    function testLzReceiveIgnoresStaleTransferTreeIndex() public {
        Hub localHub = _deployInitializedHub();
        Hub.TokenInfo memory info =
            Hub.TokenInfo({chainId: 606, eid: 88, verifier: address(0x10), token: address(0x11)});
        localHub.registerToken(info);
        localHub.setPeer(info.eid, _toBytes32(address(this)));

        Origin memory origin = Origin({srcEid: info.eid, sender: _toBytes32(address(this)), nonce: 1});
        bytes memory freshPayload = abi.encode(uint256(111), uint64(10));
        vm.prank(address(endpoint));
        localHub.lzReceive(origin, bytes32(0), freshPayload, address(0), bytes(""));

        assertEq(localHub.transferTreeIndices(0), 10, "fresh index stored");
        assertEq(localHub.transferRoots(0), 111, "fresh root stored");

        bytes memory stalePayload = abi.encode(uint256(222), uint64(5));
        vm.recordLogs();
        vm.prank(address(endpoint));
        localHub.lzReceive(origin, bytes32(0), stalePayload, address(0), bytes(""));
        Vm.Log[] memory logs = vm.getRecordedLogs();
        assertEq(logs.length, 0, "no events emitted for stale update");

        assertEq(localHub.transferTreeIndices(0), 10, "stale index ignored");
        assertEq(localHub.transferRoots(0), 111, "stale root ignored");
    }

    function testHubUpgradePreservesState() public {
        Hub implementation = new Hub(address(endpoint));
        bytes memory initData = abi.encodeCall(Hub.initialize, (address(this)));
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        Hub proxiedHub = Hub(address(proxy));

        Hub.TokenInfo memory info =
            Hub.TokenInfo({chainId: 555, eid: 505, verifier: address(0x1234), token: address(0x5678)});
        proxiedHub.registerToken(info);
        (uint64 storedChainId,, address storedVerifier, address storedToken) = proxiedHub.tokenInfos(0);
        assertEq(storedChainId, info.chainId, "state setup failed");
        assertEq(storedVerifier, info.verifier, "verifier not stored initially");
        assertEq(storedToken, info.token, "token not stored initially");

        HubUpgradeMock newImplementation = new HubUpgradeMock(address(endpoint));
        proxiedHub.upgradeToAndCall(address(newImplementation), bytes(""));

        HubUpgradeMock upgraded = HubUpgradeMock(address(proxiedHub));
        assertEq(upgraded.version(), "hub-v2", "upgraded implementation not in use");

        (uint64 chainId,, address verifierAddr, address tokenAddr) = proxiedHub.tokenInfos(0);
        assertEq(chainId, info.chainId, "chain id not preserved");
        assertEq(verifierAddr, info.verifier, "verifier not preserved");
        assertEq(tokenAddr, info.token, "token not preserved");
    }

    function testHubUpgradeRevertsOnEndpointMismatch() public {
        Hub implementation = new Hub(address(endpoint));
        bytes memory initData = abi.encodeCall(Hub.initialize, (address(this)));
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        Hub proxiedHub = Hub(address(proxy));

        EndpointV2 otherEndpoint = endpointSetup.endpointList[1];
        HubUpgradeMock newImplementation = new HubUpgradeMock(address(otherEndpoint));

        vm.expectRevert(
            abi.encodeWithSelector(Hub.EndpointMismatch.selector, address(endpoint), address(otherEndpoint))
        );
        proxiedHub.upgradeToAndCall(address(newImplementation), bytes(""));
    }

    function testRegisterTokenOnlyOwner() public {
        address nonOwner = address(0xBEEF);
        Hub.TokenInfo memory info = Hub.TokenInfo({chainId: 999, eid: 999, verifier: address(0x1), token: address(0x2)});

        vm.prank(nonOwner);
        vm.expectRevert(abi.encodeWithSelector(OwnableUpgradeable.OwnableUnauthorizedAccount.selector, nonOwner));
        hub.registerToken(info);
    }

    function testUpdateTokenOnlyOwner() public {
        address nonOwner = address(0xBEEF);
        Hub.TokenInfo memory info =
            Hub.TokenInfo({chainId: 999, eid: REMOTE_EID_A, verifier: address(0x1), token: address(0x2)});

        vm.prank(nonOwner);
        vm.expectRevert(abi.encodeWithSelector(OwnableUpgradeable.OwnableUnauthorizedAccount.selector, nonOwner));
        hub.updateToken(info);
    }

    function testUpdateTokenValidationReverts() public {
        vm.expectRevert(Hub.ZeroVerifier.selector);
        hub.updateToken(Hub.TokenInfo({chainId: 1, eid: REMOTE_EID_A, verifier: address(0), token: address(0x1)}));

        vm.expectRevert(Hub.ZeroToken.selector);
        hub.updateToken(Hub.TokenInfo({chainId: 1, eid: REMOTE_EID_A, verifier: address(0x1), token: address(0)}));

        vm.expectRevert(Hub.InvalidChainId.selector);
        hub.updateToken(Hub.TokenInfo({chainId: 0, eid: REMOTE_EID_A, verifier: address(0x1), token: address(0x2)}));
    }

    function testRegisterTokenRevertsOnMaxCapacity() public {
        Hub localHub = _deployInitializedHub();
        uint256 maxLeaves = localHub.MAX_LEAVES();

        for (uint256 i = 0; i < maxLeaves; ++i) {
            // Casts are safe because i < maxLeaves (64) fits in uint64/uint32/uint160
            // forge-lint: disable-next-line(unsafe-typecast)
            uint64 chainId = uint64(i + 1);
            // forge-lint: disable-next-line(unsafe-typecast)
            uint32 eid = uint32(i + 100);
            // forge-lint: disable-next-line(unsafe-typecast)
            address verifier = address(uint160(i + 1));
            // forge-lint: disable-next-line(unsafe-typecast)
            address token = address(uint160(i + 1000));
            localHub.registerToken(Hub.TokenInfo({chainId: chainId, eid: eid, verifier: verifier, token: token}));
        }

        vm.expectRevert(Hub.HubCapacityReached.selector);
        localHub.registerToken(Hub.TokenInfo({chainId: 999, eid: 9999, verifier: address(0x1), token: address(0x2)}));
    }

    function testZeroHashReturnsInitializedValues() public view {
        // zeroHash[0] is 0 by design (Poseidon zero leaf)
        // zeroHash[1] = hash(0, 0) which should be non-zero
        uint256 secondZeroHash = hub.zeroHash(1);
        assertGt(secondZeroHash, 0, "zero hash[1] should be non-zero");
    }

    function testGetTransferRootsAndIndicesReturnsSnapshot() public {
        Hub localHub = _deployInitializedHub();
        Hub.TokenInfo memory info1 = Hub.TokenInfo({chainId: 1, eid: 10, verifier: address(0x1), token: address(0x2)});
        Hub.TokenInfo memory info2 = Hub.TokenInfo({chainId: 2, eid: 20, verifier: address(0x3), token: address(0x4)});
        localHub.registerToken(info1);
        localHub.registerToken(info2);
        localHub.setPeer(info1.eid, _toBytes32(address(this)));
        localHub.setPeer(info2.eid, _toBytes32(address(this)));

        Origin memory origin1 = Origin({srcEid: info1.eid, sender: _toBytes32(address(this)), nonce: 1});
        vm.prank(address(endpoint));
        localHub.lzReceive(origin1, bytes32(0), abi.encode(uint256(111), uint64(5)), address(0), bytes(""));

        Origin memory origin2 = Origin({srcEid: info2.eid, sender: _toBytes32(address(this)), nonce: 1});
        vm.prank(address(endpoint));
        localHub.lzReceive(origin2, bytes32(0), abi.encode(uint256(222), uint64(10)), address(0), bytes(""));

        (uint256[] memory roots, uint64[] memory indices) = localHub.getTransferRootsAndIndices();

        assertEq(roots.length, 2, "roots length");
        assertEq(indices.length, 2, "indices length");
        assertEq(roots[0], 111, "first root");
        assertEq(roots[1], 222, "second root");
        assertEq(indices[0], 5, "first index");
        assertEq(indices[1], 10, "second index");
    }

    function testCurrentAggregationRootMatchesBroadcast() public {
        Hub localHub = _deployInitializedHub();
        Hub.TokenInfo memory info = Hub.TokenInfo({chainId: 1, eid: 10, verifier: address(0x1), token: address(0x2)});
        localHub.registerToken(info);
        localHub.setPeer(info.eid, _toBytes32(address(this)));

        Origin memory origin = Origin({srcEid: info.eid, sender: _toBytes32(address(this)), nonce: 1});
        vm.prank(address(endpoint));
        localHub.lzReceive(origin, bytes32(0), abi.encode(uint256(12345), uint64(1)), address(0), bytes(""));

        uint256 computedRoot = localHub.currentAggregationRoot();
        assertGt(computedRoot, 0, "aggregation root should be non-zero");
    }

    function testLzReceiveIgnoresSameTransferTreeIndex() public {
        Hub localHub = _deployInitializedHub();
        Hub.TokenInfo memory info = Hub.TokenInfo({chainId: 1, eid: 10, verifier: address(0x1), token: address(0x2)});
        localHub.registerToken(info);
        localHub.setPeer(info.eid, _toBytes32(address(this)));

        Origin memory origin = Origin({srcEid: info.eid, sender: _toBytes32(address(this)), nonce: 1});
        vm.prank(address(endpoint));
        localHub.lzReceive(origin, bytes32(0), abi.encode(uint256(111), uint64(5)), address(0), bytes(""));

        assertEq(localHub.transferRoots(0), 111, "initial root stored");

        vm.recordLogs();
        vm.prank(address(endpoint));
        localHub.lzReceive(origin, bytes32(0), abi.encode(uint256(222), uint64(5)), address(0), bytes(""));
        Vm.Log[] memory logs = vm.getRecordedLogs();

        assertEq(logs.length, 0, "no events for same index");
        assertEq(localHub.transferRoots(0), 111, "root unchanged for same index");
    }

    function testBroadcastEmitsAggregationRootUpdated() public {
        // Use existing hub which has sendLib configured
        // First update a transfer root via lzReceive
        Origin memory origin = Origin({srcEid: REMOTE_EID_A, sender: _toBytes32(REMOTE_PEER_A), nonce: 1});
        vm.prank(address(endpoint));
        hub.lzReceive(origin, bytes32(0), abi.encode(uint256(12345), uint64(1)), address(0), bytes(""));

        uint32[] memory targetEids = _targetEids();
        bytes memory options = _options();

        uint256 fee = hub.quoteBroadcast(targetEids, options);
        vm.deal(address(this), fee);

        vm.recordLogs();
        hub.broadcast{value: fee}(targetEids, options);
        Vm.Log[] memory logs = vm.getRecordedLogs();

        bool foundEvent = false;
        bytes32 eventSig = keccak256("AggregationRootUpdated(uint256,uint64,uint256[],uint64[])");
        for (uint256 i = 0; i < logs.length; ++i) {
            if (logs[i].topics[0] == eventSig) {
                foundEvent = true;
                uint256 emittedRoot = uint256(logs[i].topics[1]);
                uint64 emittedSeq = uint64(uint256(logs[i].topics[2]));
                assertGt(emittedRoot, 0, "emitted root non-zero");
                assertEq(emittedSeq, 1, "emitted seq");
            }
        }
        assertTrue(foundEvent, "AggregationRootUpdated event emitted");
    }

    function _targetEids() internal pure returns (uint32[] memory targetEids) {
        targetEids = new uint32[](2);
        targetEids[0] = REMOTE_EID_A;
        targetEids[1] = REMOTE_EID_B;
    }

    function _deployInitializedHub() internal returns (Hub deployedHub) {
        Hub implementation = new Hub(address(endpoint));
        bytes memory initData = abi.encodeCall(Hub.initialize, (address(this)));
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        deployedHub = Hub(address(proxy));
    }

    function _deployInitializedZeroRootHub() internal returns (Hub deployedHub) {
        Hub implementation = new HubZeroRootMock(address(endpoint));
        bytes memory initData = abi.encodeCall(Hub.initialize, (address(this)));
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        deployedHub = Hub(address(proxy));
    }

    function _options() internal pure returns (bytes memory options) {
        options = OptionsBuilder.newOptions();
        options = options.addExecutorLzReceiveOption(200_000, 0);
    }

    function _toBytes32(address addr) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(addr)));
    }
}

contract HubUpgradeMock is Hub {
    constructor(address endpoint) Hub(endpoint) {}

    function version() external pure returns (string memory) {
        return "hub-v2";
    }
}

contract HubZeroRootMock is Hub {
    constructor(address endpoint) Hub(endpoint) {}

    function _computeAggregationRoot(uint256[] memory, uint256[ZERO_HASH_COUNT] memory)
        internal
        pure
        override
        returns (uint256)
    {
        return 0;
    }
}
