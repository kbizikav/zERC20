// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Vm} from "forge-std/Vm.sol";
import {Hub} from "../src/Hub.sol";
import {Origin} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oapp/OApp.sol";
import {OptionsBuilder} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oapp/libs/OptionsBuilder.sol";
import {TestHelperOz5, EndpointV2Mock, MockSendLib} from "./utils/TestHelperOz5.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";

contract HubTest is TestHelperOz5 {
    using OptionsBuilder for bytes;

    Hub internal hub;
    EndpointV2Mock internal endpoint;
    MockSendLib internal sendLib;

    uint32 internal constant LOCAL_EID = 1;
    uint32 internal constant REMOTE_EID_A = 2;
    uint32 internal constant REMOTE_EID_B = 3;
    uint256 internal constant FEE_PER_MESSAGE = 0.05 ether;

    address internal constant REMOTE_PEER_A = address(0xAA);
    address internal constant REMOTE_PEER_B = address(0xBB);

    bytes32 internal constant PACKET_SENT_SIG = keccak256("PacketSent(bytes,bytes,address)");

    receive() external payable {}

    function setUp() public {
        endpoint = _deployEndpoint(LOCAL_EID);
        sendLib = _deployMessageLib();

        endpoint.registerLibrary(address(sendLib));
        endpoint.setDefaultSendLibrary(REMOTE_EID_A, address(sendLib));
        endpoint.setDefaultSendLibrary(REMOTE_EID_B, address(sendLib));
        endpoint.setDefaultReceiveLibrary(REMOTE_EID_A, address(sendLib), 0);
        endpoint.setDefaultReceiveLibrary(REMOTE_EID_B, address(sendLib), 0);
        endpoint.setMessagingFee(REMOTE_EID_A, FEE_PER_MESSAGE, 0);
        endpoint.setMessagingFee(REMOTE_EID_B, FEE_PER_MESSAGE, 0);

        hub = _deployInitializedHub();

        hub.setPeer(REMOTE_EID_A, _toBytes32(REMOTE_PEER_A));
        hub.setPeer(REMOTE_EID_B, _toBytes32(REMOTE_PEER_B));

        hub.registerToken(Hub.TokenInfo({chainId: 101, eid: REMOTE_EID_A, verifier: address(0x1), token: address(0x2)}));
        hub.registerToken(Hub.TokenInfo({chainId: 202, eid: REMOTE_EID_B, verifier: address(0x3), token: address(0x4)}));
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
                packetCount++;
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
        assertFalse(localHub.isUpToDate(), "hub marked stale after registration");
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

    function testLzReceiveUpdatesRootAndFlags() public {
        Hub localHub = _deployInitializedHub();
        Hub.TokenInfo memory info =
            Hub.TokenInfo({chainId: 909, eid: 77, verifier: address(0x7), token: address(0x8)});
        localHub.registerToken(info);

        uint32[] memory empty = new uint32[](0);
        bytes memory emptyOptions = bytes("");
        localHub.broadcast(empty, emptyOptions);
        assertTrue(localHub.isUpToDate(), "precondition up to date");

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
        assertFalse(localHub.isUpToDate(), "hub marked stale");
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
        Hub implementation = new Hub();
        bytes memory initData = abi.encodeCall(Hub.initialize, (address(endpoint), address(this)));
        ERC1967Proxy proxy = new ERC1967Proxy(address(implementation), initData);
        Hub proxiedHub = Hub(address(proxy));

        Hub.TokenInfo memory info =
            Hub.TokenInfo({chainId: 555, eid: 505, verifier: address(0x1234), token: address(0x5678)});
        proxiedHub.registerToken(info);
        (uint64 storedChainId,, address storedVerifier, address storedToken) = proxiedHub.tokenInfos(0);
        assertEq(storedChainId, info.chainId, "state setup failed");
        assertEq(storedVerifier, info.verifier, "verifier not stored initially");
        assertEq(storedToken, info.token, "token not stored initially");

        HubUpgradeMock newImplementation = new HubUpgradeMock();
        proxiedHub.upgradeTo(address(newImplementation));

        HubUpgradeMock upgraded = HubUpgradeMock(address(proxiedHub));
        assertEq(upgraded.version(), "hub-v2", "upgraded implementation not in use");

        (uint64 chainId,, address verifierAddr, address tokenAddr) = proxiedHub.tokenInfos(0);
        assertEq(chainId, info.chainId, "chain id not preserved");
        assertEq(verifierAddr, info.verifier, "verifier not preserved");
        assertEq(tokenAddr, info.token, "token not preserved");
    }

    function _targetEids() internal pure returns (uint32[] memory targetEids) {
        targetEids = new uint32[](2);
        targetEids[0] = REMOTE_EID_A;
        targetEids[1] = REMOTE_EID_B;
    }

    function _deployInitializedHub() internal returns (Hub deployedHub) {
        Hub implementation = new Hub();
        bytes memory initData = abi.encodeCall(Hub.initialize, (address(endpoint), address(this)));
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
    function version() external pure returns (string memory) {
        return "hub-v2";
    }
}
