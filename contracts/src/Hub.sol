// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.30;

import {MessagingFee} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oapp/OAppSender.sol";
import {Origin} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oapp/OApp.sol";
import {PoseidonAggregationLib} from "./utils/PoseidonAggregationLib.sol";
import {POSEIDON_ZERO_HASH_COUNT, POSEIDON_MAX_LEAVES} from "./utils/PoseidonAggregationConfig.sol";
import {OAppUpgradeable} from "./utils/layerzero/OAppUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {SlotDerivation} from "@openzeppelin/contracts/utils/SlotDerivation.sol";

/**
 * @title Hub
 * @notice Central LayerZero OApp that tracks per-token transfer roots, Poseidon-aggregates them, and broadcasts the
 *         global root consumed by verifier contracts.
 * @dev Implements the PoseidonT3 tree (height 6 / 64 leaves) and fee semantics consumed by the verifier network.
 */
contract Hub is OAppUpgradeable, UUPSUpgradeable {
    using SlotDerivation for string;

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
    error EmptyTargetEids();

    /// -----------------------------------------------------------------------
    /// Constants & Storage
    /// -----------------------------------------------------------------------

    uint256 public constant MAX_LEAVES = POSEIDON_MAX_LEAVES;
    uint256 internal constant ZERO_HASH_COUNT = POSEIDON_ZERO_HASH_COUNT;
    uint256 internal constant TRANSFER_PAYLOAD_LENGTH = 64;

    /// @custom:storage-location erc7201:zerc20.storage.hub
    struct HubStorage {
        uint256[] transferRoots;
        uint64[] transferTreeIndices;
        TokenInfo[] tokenInfos;
        mapping(uint32 => uint256) eidToPosition; // 1-based index, 0 means unregistered
        uint256[ZERO_HASH_COUNT] zeroHash;
        uint64 aggSeq;
    }

    function _getHubStorage() private pure returns (HubStorage storage $) {
        bytes32 slot = SlotDerivation.erc7201Slot("zerc20.storage.hub");
        assembly {
            $.slot := slot
        }
    }

    /// -----------------------------------------------------------------------
    /// Constructor
    /// -----------------------------------------------------------------------

    constructor() {
        _disableInitializers();
    }

    /// @notice Initializes the Hub's LayerZero endpoint/delegate pairing alongside upgrade hooks.
    /// @param endpoint LayerZero endpoint used for cross-chain messaging.
    /// @param delegate Address that MUST become both the contract owner and LayerZero delegate so admin controls and callbacks share the same authority.
    function initialize(address endpoint, address delegate) external initializer {
        __OApp_init(endpoint, delegate);
        __UUPSUpgradeable_init();
        __Hub_init();
    }

    function _authorizeUpgrade(address) internal override onlyOwner {}

    function transferRoots(uint256 index_) public view returns (uint256) {
        return _getHubStorage().transferRoots[index_];
    }

    function transferTreeIndices(uint256 index_) public view returns (uint64) {
        return _getHubStorage().transferTreeIndices[index_];
    }

    function tokenInfos(uint256 index_)
        public
        view
        returns (uint64 chainId, uint32 eid, address verifier, address token)
    {
        TokenInfo storage info = _getHubStorage().tokenInfos[index_];
        return (info.chainId, info.eid, info.verifier, info.token);
    }

    function eidToPosition(uint32 eid) public view returns (uint256) {
        return _getHubStorage().eidToPosition[eid];
    }

    function zeroHash(uint256 index_) public view returns (uint256) {
        return _getHubStorage().zeroHash[index_];
    }

    function aggSeq() public view returns (uint64) {
        return _getHubStorage().aggSeq;
    }

    /// forge-lint: disable-next-line(mixed-case-function)
    function __Hub_init() internal onlyInitializing {
        HubStorage storage $ = _getHubStorage();
        uint256[ZERO_HASH_COUNT] memory zeroHashInit = PoseidonAggregationLib.generateZeroHashes();
        for (uint256 i = 0; i < zeroHashInit.length; ++i) {
            $.zeroHash[i] = zeroHashInit[i];
        }
    }

    /// -----------------------------------------------------------------------
    /// Mutations
    /// -----------------------------------------------------------------------

    /// @notice Registers a token/verifier tuple so its transfer roots contribute to the Poseidon aggregation tree.
    /// @dev Mirrors the owner-gated registration flow defined for the aggregation hub.
    /// @param info Struct containing chainId, endpoint ID, verifier, and token metadata.
    function registerToken(TokenInfo calldata info) external onlyOwner {
        if (info.verifier == address(0)) revert ZeroVerifier();
        if (info.token == address(0)) revert ZeroToken();
        if (info.chainId == 0) revert InvalidChainId();
        HubStorage storage $ = _getHubStorage();
        if ($.eidToPosition[info.eid] != 0) revert TokenAlreadyRegistered(info.eid);
        if ($.transferRoots.length >= MAX_LEAVES) revert HubCapacityReached();

        uint256 index = $.transferRoots.length;
        $.transferRoots.push(0);
        $.transferTreeIndices.push(0);
        $.tokenInfos.push(info);
        $.eidToPosition[info.eid] = index + 1;

        emit TokenRegistered(info.eid, index, info.chainId, info.token, info.verifier);
    }

    /// @notice Refreshes the verifier/token metadata for an already-registered endpoint without reordering leaves.
    /// @param info Updated metadata sharing the same `eid` slot.
    function updateToken(TokenInfo calldata info) external onlyOwner {
        if (info.verifier == address(0)) revert ZeroVerifier();
        if (info.token == address(0)) revert ZeroToken();
        if (info.chainId == 0) revert InvalidChainId();

        HubStorage storage $ = _getHubStorage();
        uint256 pos = $.eidToPosition[info.eid];
        if (pos == 0) revert TokenNotRegistered(info.eid);

        uint256 index = pos - 1;
        $.tokenInfos[index] = info;

        emit TokenUpdated(info.eid, index, info.chainId, info.token, info.verifier);
    }

    /// @notice Snapshots transfer roots, computes the Poseidon aggregation root, and broadcasts it to target EIDs.
    /// @dev Implements the `broadcast` step, including fee refund semantics.
    /// @param targetEids LayerZero endpoint IDs that must receive the global root.
    /// @param lzOptions LayerZero execution parameters (gas, native drop, etc.).
    function broadcast(uint32[] calldata targetEids, bytes calldata lzOptions) external payable {
        if (targetEids.length == 0) revert EmptyTargetEids();
        BroadcastContext memory ctx = _computeBroadcastContext();
        bytes memory options = lzOptions;
        MessagingFee[] memory fees = new MessagingFee[](targetEids.length);
        uint256 totalNativeFee = _quoteBroadcast(targetEids, ctx.payload, options, fees);

        if (msg.value < totalNativeFee) revert NativeFeeMismatch(msg.value, totalNativeFee);
        uint256 refund = msg.value - totalNativeFee;

        _getHubStorage().aggSeq = ctx.nextAggSeq;

        for (uint256 i = 0; i < targetEids.length; ++i) {
            _lzSend(targetEids[i], ctx.payload, options, fees[i], msg.sender);
        }

        if (refund != 0) {
            (bool success,) = msg.sender.call{value: refund}("");
            if (!success) revert FeeRefundFailed(refund);
        }

        emit AggregationRootUpdated(ctx.aggregationRoot, ctx.nextAggSeq, ctx.snapshot, ctx.transferTreeIndicesSnapshot);
    }

    /// @notice Estimates the total native fee required to broadcast a payload to the provided endpoints.
    /// @param targetEids Destination endpoint IDs that must already be registered.
    /// @param lzOptions LayerZero execution parameters mirrored from `broadcast`.
    /// @return totalNativeFee Sum of endpoint-specific native fee quotes.
    function quoteBroadcast(uint32[] calldata targetEids, bytes calldata lzOptions)
        external
        view
        returns (uint256 totalNativeFee)
    {
        if (targetEids.length == 0) revert EmptyTargetEids();
        bytes memory options = lzOptions;
        bytes memory dummyPayload = abi.encode(uint256(0), 1);
        totalNativeFee = _quoteBroadcast(targetEids, dummyPayload, options, new MessagingFee[](targetEids.length));
    }

    /// @notice Returns a memory copy of the registered token metadata for off-chain auditors.
    function getTokenInfos() external view returns (TokenInfo[] memory infos) {
        HubStorage storage $ = _getHubStorage();
        uint256 len = $.tokenInfos.length;
        infos = new TokenInfo[](len);
        for (uint256 i = 0; i < len; ++i) {
            infos[i] = $.tokenInfos[i];
        }
    }

    /// @notice Exposes the latest per-token transfer roots along with their monotonically increasing tree indices.
    function getTransferRootsAndIndices() external view returns (uint256[] memory roots, uint64[] memory treeIndices) {
        HubStorage storage $ = _getHubStorage();
        uint256 len = $.transferRoots.length;
        roots = new uint256[](len);
        treeIndices = new uint64[](len);
        for (uint256 i = 0; i < len; ++i) {
            roots[i] = $.transferRoots[i];
            treeIndices[i] = $.transferTreeIndices[i];
        }
    }

    /// @notice Computes the aggregation root for the current transfer roots without mutating state.
    /// @dev Mirrors the tree calculation performed inside `_computeBroadcastContext` so off-chain agents can poll freshness.
    /// @return aggregationRoot The Poseidon aggregation root derived from the latest transfer roots snapshot.
    function currentAggregationRoot() external view returns (uint256 aggregationRoot) {
        HubStorage storage $ = _getHubStorage();
        uint256 len = $.transferRoots.length;
        uint256[] memory leaves = new uint256[](len);
        for (uint256 i = 0; i < len; ++i) {
            leaves[i] = $.transferRoots[i];
        }

        uint256[ZERO_HASH_COUNT] memory zeroHashCache = $.zeroHash;
        aggregationRoot = PoseidonAggregationLib.computeAggregationRoot(leaves, zeroHashCache);
    }

    /// @dev Copies current leaves, computes the Poseidon aggregation root, and prepares the outbound payload/seq.
    function _computeBroadcastContext() internal view returns (BroadcastContext memory ctx) {
        HubStorage storage $ = _getHubStorage();
        uint256 len = $.transferRoots.length;
        ctx.snapshot = new uint256[](len);
        ctx.transferTreeIndicesSnapshot = new uint64[](len);
        uint256[] memory leaves = new uint256[](len);
        for (uint256 i = 0; i < len; ++i) {
            uint256 root = $.transferRoots[i];
            ctx.snapshot[i] = root;
            leaves[i] = root;
            ctx.transferTreeIndicesSnapshot[i] = $.transferTreeIndices[i];
        }

        uint256[ZERO_HASH_COUNT] memory zeroHashCache = $.zeroHash;
        ctx.aggregationRoot = PoseidonAggregationLib.computeAggregationRoot(leaves, zeroHashCache);
        ctx.nextAggSeq = $.aggSeq + 1;
        ctx.payload = abi.encode(ctx.aggregationRoot, ctx.nextAggSeq);
    }

    /// @dev Internal helper shared by `broadcast` and `quoteBroadcast` that enforces registration and native-fee-only.
    function _quoteBroadcast(
        uint32[] calldata targetEids,
        bytes memory payload,
        bytes memory options,
        MessagingFee[] memory fees
    ) internal view returns (uint256 totalNativeFee) {
        HubStorage storage $ = _getHubStorage();
        uint256 len = targetEids.length;
        for (uint256 i = 0; i < len; ++i) {
            uint32 eid = targetEids[i];
            if ($.eidToPosition[eid] == 0) revert TokenNotRegistered(eid);
            MessagingFee memory fee = _quote(eid, payload, options, false);
            if (fee.lzTokenFee != 0) revert LayerZeroTokenFeeUnsupported(eid, fee.lzTokenFee);
            fees[i] = fee;
            totalNativeFee += fee.nativeFee;
        }
    }

    /// -----------------------------------------------------------------------
    /// LayerZero Receiver
    /// -----------------------------------------------------------------------

    /// @dev Accepts `(transferRoot, transferTreeIndex)` payloads from registered verifiers when indices advance.
    function _lzReceive(Origin calldata origin, bytes32, bytes calldata payload, address, bytes calldata)
        internal
        override
    {
        HubStorage storage $ = _getHubStorage();
        uint256 pos = $.eidToPosition[origin.srcEid];
        if (pos == 0) revert TokenNotRegistered(origin.srcEid);

        if (payload.length != TRANSFER_PAYLOAD_LENGTH) revert InvalidPayloadLength(payload.length);

        (uint256 transferRoot, uint64 transferTreeIndex) = abi.decode(payload, (uint256, uint64));
        uint256 index = pos - 1;
        uint64 currentTreeIndex = $.transferTreeIndices[index];
        if (transferTreeIndex <= currentTreeIndex) {
            return;
        }
        $.transferRoots[index] = transferRoot;
        $.transferTreeIndices[index] = transferTreeIndex;
        emit TransferRootUpdated(origin.srcEid, index, transferRoot);
    }

    function _payNative(uint256 _nativeFee) internal override returns (uint256 nativeFee) {
        if (msg.value < _nativeFee) revert NotEnoughNative(msg.value);
        return _nativeFee;
    }
}
