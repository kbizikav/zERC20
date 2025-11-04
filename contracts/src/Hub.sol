// SPDX-License-Identifier: Unlicense
pragma solidity ^0.8.20;

import {MessagingFee} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oapp/OAppSender.sol";
import {Origin} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oapp/OApp.sol";
import {PoseidonAggregationLib} from "./utils/PoseidonAggregationLib.sol";
import {POSEIDON_ZERO_HASH_COUNT, POSEIDON_MAX_LEAVES} from "./utils/PoseidonAggregationConfig.sol";
import {OAppUpgradeable} from "./utils/layerzero/OAppUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";

/**
 * @title Hub
 * @notice Maintains per-EID transfer roots, aggregates them via Poseidon, and broadcasts the aggregation root.
 * @dev Matches docs/contract_jp.md with PoseidonT3 hashing and a maximum of MAX_LEAVES leaves.
 */
contract Hub is OAppUpgradeable, UUPSUpgradeable {
    /// -----------------------------------------------------------------------
    /// Structs / Events
    /// -----------------------------------------------------------------------

    struct TokenInfo {
        uint64 chainId;
        uint32 eid;
        address verifier;
        address token;
    }

    struct BroadcastContext {
        uint256[] snapshot;
        uint64[] transferTreeIndicesSnapshot;
        uint256 aggregationRoot;
        uint64 nextAggSeq;
        bytes payload;
    }

    event TokenRegistered(uint32 indexed eid, uint256 indexed index, uint64 chainId, address token, address verifier);
    event TokenUpdated(uint32 indexed eid, uint256 indexed index, uint64 chainId, address token, address verifier);
    event TransferRootUpdated(uint32 indexed eid, uint256 indexed index, uint256 newRoot);
    event AggregationRootUpdated(
        uint256 indexed root,
        uint64 indexed aggSeq,
        uint256[] transferRootsSnapshot,
        uint64[] transferTreeIndicesSnapshot
    );

    /// -----------------------------------------------------------------------
    /// Errors
    /// -----------------------------------------------------------------------

    error HubCapacityReached();
    error TokenAlreadyRegistered(uint32 eid);
    error TokenNotRegistered(uint32 eid);
    error ZeroVerifier();
    error ZeroToken();
    error InvalidChainId();
    error InvalidPayloadLength(uint256 length);
    error NativeFeeMismatch(uint256 provided, uint256 required);
    error LayerZeroTokenFeeUnsupported(uint32 eid, uint256 lzTokenFee);
    error FeeRefundFailed(uint256 amount);

    /// -----------------------------------------------------------------------
    /// Constants & Storage
    /// -----------------------------------------------------------------------

    uint256 public constant MAX_LEAVES = POSEIDON_MAX_LEAVES;
    uint256 internal constant ZERO_HASH_COUNT = POSEIDON_ZERO_HASH_COUNT;
    uint256 internal constant TRANSFER_PAYLOAD_LENGTH = 64;

    uint256[] public transferRoots;
    uint64[] public transferTreeIndices;
    TokenInfo[] public tokenInfos;

    mapping(uint32 => uint256) public eidToPosition; // 1-based index, 0 means unregistered
    uint256[ZERO_HASH_COUNT] public zeroHash;
    uint64 public aggSeq;
    bool public isUpToDate;

    /// -----------------------------------------------------------------------
    /// Constructor
    /// -----------------------------------------------------------------------

    constructor() {
        _disableInitializers();
    }

    function initialize(address endpoint, address delegate) external initializer {
        __OApp_init(endpoint, delegate);
        __UUPSUpgradeable_init();
        __Hub_init();
    }

    function _authorizeUpgrade(address) internal override onlyOwner {}

    /// forge-lint: disable-next-line(mixed-case-function)
    function __Hub_init() internal onlyInitializing {
        uint256[ZERO_HASH_COUNT] memory zeroHashInit = PoseidonAggregationLib.generateZeroHashes();
        for (uint256 i = 0; i < zeroHashInit.length; ++i) {
            zeroHash[i] = zeroHashInit[i];
        }
        isUpToDate = true;
    }

    /// -----------------------------------------------------------------------
    /// Mutations
    /// -----------------------------------------------------------------------

    function registerToken(TokenInfo calldata info) external onlyOwner {
        if (info.verifier == address(0)) revert ZeroVerifier();
        if (info.token == address(0)) revert ZeroToken();
        if (info.chainId == 0) revert InvalidChainId();
        if (eidToPosition[info.eid] != 0) revert TokenAlreadyRegistered(info.eid);
        if (transferRoots.length >= MAX_LEAVES) revert HubCapacityReached();

        uint256 index = transferRoots.length;
        transferRoots.push(0);
        transferTreeIndices.push(0);
        tokenInfos.push(info);
        eidToPosition[info.eid] = index + 1;
        isUpToDate = false;

        emit TokenRegistered(info.eid, index, info.chainId, info.token, info.verifier);
    }

    function updateToken(TokenInfo calldata info) external onlyOwner {
        if (info.verifier == address(0)) revert ZeroVerifier();
        if (info.token == address(0)) revert ZeroToken();
        if (info.chainId == 0) revert InvalidChainId();

        uint256 pos = eidToPosition[info.eid];
        if (pos == 0) revert TokenNotRegistered(info.eid);

        uint256 index = pos - 1;
        tokenInfos[index] = info;

        emit TokenUpdated(info.eid, index, info.chainId, info.token, info.verifier);
    }

    function broadcast(uint32[] calldata targetEids, bytes calldata lzOptions) public payable {
        BroadcastContext memory ctx = _computeBroadcastContext();
        bytes memory options = lzOptions;
        MessagingFee[] memory fees = new MessagingFee[](targetEids.length);
        uint256 totalNativeFee = _quoteBroadcast(targetEids, ctx.payload, options, fees);

        if (msg.value < totalNativeFee) revert NativeFeeMismatch(msg.value, totalNativeFee);
        uint256 refund = msg.value - totalNativeFee;

        aggSeq = ctx.nextAggSeq;

        for (uint256 i = 0; i < targetEids.length; ++i) {
            _lzSend(targetEids[i], ctx.payload, options, fees[i], msg.sender);
        }

        if (refund != 0) {
            (bool success,) = msg.sender.call{value: refund}("");
            if (!success) revert FeeRefundFailed(refund);
        }

        isUpToDate = true;
        emit AggregationRootUpdated(ctx.aggregationRoot, ctx.nextAggSeq, ctx.snapshot, ctx.transferTreeIndicesSnapshot);
    }

    function quoteBroadcast(uint32[] calldata targetEids, bytes calldata lzOptions)
        public
        view
        returns (uint256 totalNativeFee)
    {
        bytes memory options = lzOptions;
        bytes memory dummyPayload = abi.encode(uint256(0), 1);
        totalNativeFee = _quoteBroadcast(targetEids, dummyPayload, options, new MessagingFee[](targetEids.length));
    }

    function getTokenInfos() external view returns (TokenInfo[] memory infos) {
        uint256 len = tokenInfos.length;
        infos = new TokenInfo[](len);
        for (uint256 i = 0; i < len; ++i) {
            infos[i] = tokenInfos[i];
        }
    }

    function _computeBroadcastContext() internal view returns (BroadcastContext memory ctx) {
        uint256 len = transferRoots.length;
        ctx.snapshot = new uint256[](len);
        ctx.transferTreeIndicesSnapshot = new uint64[](len);
        uint256[] memory leaves = new uint256[](len);
        for (uint256 i = 0; i < len; ++i) {
            uint256 root = transferRoots[i];
            ctx.snapshot[i] = root;
            leaves[i] = root;
            ctx.transferTreeIndicesSnapshot[i] = transferTreeIndices[i];
        }

        uint256[ZERO_HASH_COUNT] memory zeroHashCache = zeroHash;
        ctx.aggregationRoot = PoseidonAggregationLib.computeAggregationRoot(leaves, zeroHashCache);
        ctx.nextAggSeq = aggSeq + 1;
        ctx.payload = abi.encode(ctx.aggregationRoot, ctx.nextAggSeq);
    }

    function _quoteBroadcast(
        uint32[] calldata targetEids,
        bytes memory payload,
        bytes memory options,
        MessagingFee[] memory fees
    ) internal view returns (uint256 totalNativeFee) {
        uint256 len = targetEids.length;
        for (uint256 i = 0; i < len; ++i) {
            uint32 eid = targetEids[i];
            if (eidToPosition[eid] == 0) revert TokenNotRegistered(eid);
            MessagingFee memory fee = _quote(eid, payload, options, false);
            if (fee.lzTokenFee != 0) revert LayerZeroTokenFeeUnsupported(eid, fee.lzTokenFee);
            fees[i] = fee;
            totalNativeFee += fee.nativeFee;
        }
    }

    /// -----------------------------------------------------------------------
    /// LayerZero Receiver
    /// -----------------------------------------------------------------------

    function lzReceive(
        Origin calldata origin,
        bytes32 guid,
        bytes calldata payload,
        address executor,
        bytes calldata extraData
    ) public payable override {
        if (address(endpoint) != msg.sender) revert OnlyEndpoint(msg.sender);

        if (eidToPosition[origin.srcEid] == 0) revert TokenNotRegistered(origin.srcEid);
        if (payload.length != TRANSFER_PAYLOAD_LENGTH) revert InvalidPayloadLength(payload.length);

        bytes32 peer = peers[origin.srcEid];
        if (peer != bytes32(0)) {
            if (peer != origin.sender) revert OnlyPeer(origin.srcEid, origin.sender);
        } else {
            bytes32 ownerBytes = bytes32(uint256(uint160(owner())));
            if (origin.sender != ownerBytes) revert NoPeer(origin.srcEid);
        }

        _lzReceive(origin, guid, payload, executor, extraData);
    }

    function _lzReceive(Origin calldata origin, bytes32, bytes calldata payload, address, bytes calldata)
        internal
        override
    {
        uint256 pos = eidToPosition[origin.srcEid];
        if (pos == 0) revert TokenNotRegistered(origin.srcEid);

        if (payload.length != TRANSFER_PAYLOAD_LENGTH) revert InvalidPayloadLength(payload.length);

        (uint256 transferRoot, uint64 transferTreeIndex) = abi.decode(payload, (uint256, uint64));
        uint256 index = pos - 1;
        uint256 previousRoot = transferRoots[index];
        transferRoots[index] = transferRoot;
        transferTreeIndices[index] = transferTreeIndex;
        if (previousRoot != transferRoot) {
            isUpToDate = false;
        }
        emit TransferRootUpdated(origin.srcEid, index, transferRoot);
    }

    function _payNative(uint256 _nativeFee) internal override returns (uint256 nativeFee) {
        if (msg.value < _nativeFee) revert NotEnoughNative(msg.value);
        return _nativeFee;
    }
}
