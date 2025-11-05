// SPDX-License-Identifier: Unlicense
pragma solidity ^0.8.20;

import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/security/PausableUpgradeable.sol";
import {MessagingFee, MessagingReceipt} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oapp/OAppSender.sol";
import {Origin} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oapp/OApp.sol";
import {IzERC20} from "./interfaces/IzERC20.sol";
import {IRootDecider, IWithdrawDecider} from "./interfaces/IDecider.sol";
import {IWithdrawVerifier} from "./interfaces/IVerifier.sol";
import {GeneralRecipientLib} from "./utils/GeneralRecipientLib.sol";
import {OAppUpgradeable} from "./utils/layerzero/OAppUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";

/**
 * @title Verifier
 * @notice LayerZero OApp that enforces the Nova / Groth16 teleport flows,
 *         acting as the bridge between on-chain hash-chain checkpoints, cross-chain aggregation roots, and zERC20 mints.
 * @dev Tracks reserved hash chains, proved transfer roots, global aggregation roots, and cumulative teleported totals.
 */
contract Verifier is OAppUpgradeable, PausableUpgradeable, UUPSUpgradeable {
    using GeneralRecipientLib for GeneralRecipientLib.GeneralRecipient;

    event HashChainReserved(uint64 indexed index, uint256 hashChain);
    event TransferRootProved(uint64 indexed index, uint256 root);
    event TransferRootRelayed(uint64 indexed index, uint256 root, bytes lzMsgId);
    event GlobalRootSaved(uint64 indexed aggSeq, uint256 root);
    event EmergencyTriggered(uint64 indexed index, uint256 root1, uint256 root2);
    event DeactivateEmergency();
    event Teleport(address indexed to, uint256 value);
    event VerifiersSet(
        address rootDecider,
        address withdrawGlobalDecider,
        address withdrawLocalDecider,
        address singleWithdrawGlobalVerifier,
        address singleWithdrawLocalVerifier
    );

    error InvalidProof();
    error NoProvedRoot();
    error ZeroAddress();
    error InvalidHubSource(uint32 srcEid);
    error ZeroToken();
    error OldRootZero(uint64 index);
    error OldRootMismatch(uint64 index, uint256 expected, uint256 actual);
    error ReserveHashChainNotFound(uint64 index);
    error NewHashChainMismatch(uint64 index, uint256 expected, uint256 actual);
    error InvalidInitialLastLeafIndex(uint256 value);
    error InvalidInitialTotalValue(uint256 value);
    error FinalTransferRootMismatch(uint256 expected, uint256 actual);
    error FinalRecipientMismatch(uint256 expected, uint256 actual);
    error ExpectedRootZero(uint64 rootHint);
    error TransferRootMismatch(uint256 expected, uint256 actual);
    error RecipientMismatch(uint256 expected, uint256 actual);
    error InvalidRecipientChainId(uint64 provided, uint64 expected);
    error NothingToWithdraw(uint256 currentTotal, uint256 totalValue);
    error InsufficientMsgValue(uint256 required, uint256 provided);

    uint256 constant INITIAL_TRANSFER_ROOT =
        8687547638004116013653730449839507042090717944911454416140763808366589487233;

    address private _token;
    uint32 private _hubEid;

    address public rootDecider;
    address public withdrawGlobalDecider;
    address public withdrawLocalDecider;
    address public singleWithdrawGlobalVerifier;
    address public singleWithdrawLocalVerifier;

    uint64 public latestReservedIndex;
    uint64 public latestProvedIndex;
    uint64 public latestAggSeq;
    uint64 public latestRelayedIndex;

    mapping(uint64 => uint256) public reservedHashChains;
    mapping(uint64 => uint256) public provedTransferRoots;
    mapping(uint64 => uint256) public globalTransferRoots;
    mapping(uint256 => uint256) public totalTeleported;

    function token() public view returns (address) {
        return _token;
    }

    function hubEid() public view returns (uint32) {
        return _hubEid;
    }

    constructor() {
        _disableInitializers();
    }

    /// @notice Initializes the verifier with the zERC20 token, Hub endpoint, LayerZero endpoint, and initial deciders.
    /// @param token_ zERC20 token whose hash chain is reserved/minted against.
    /// @param hubEid_ LayerZero endpoint ID of the Hub contract.
    /// @param endpoint LayerZero endpoint address (forwarded to OApp init).
    /// @param delegate Relayer address allowed to execute LayerZero callbacks.
    /// @param rootDecider_ Nova verifier for transfer-root transitions.
    /// @param withdrawGlobalDecider_ Nova verifier for global teleport proofs.
    /// @param withdrawLocalDecider_ Nova verifier for local teleport proofs.
    /// @param singleWithdrawGlobalVerifier_ Groth16 verifier for global single teleports.
    /// @param singleWithdrawLocalVerifier_ Groth16 verifier for local single teleports.
    function initialize(
        address token_,
        uint32 hubEid_,
        address endpoint,
        address delegate,
        address rootDecider_,
        address withdrawGlobalDecider_,
        address withdrawLocalDecider_,
        address singleWithdrawGlobalVerifier_,
        address singleWithdrawLocalVerifier_
    ) external initializer {
        if (token_ == address(0)) revert ZeroToken();
        if (
            rootDecider_ == address(0) || withdrawGlobalDecider_ == address(0) || withdrawLocalDecider_ == address(0)
                || singleWithdrawGlobalVerifier_ == address(0) || singleWithdrawLocalVerifier_ == address(0)
        ) {
            revert ZeroAddress();
        }

        __OApp_init(endpoint, delegate);
        __UUPSUpgradeable_init();
        __Verifier_init(
            token_,
            hubEid_,
            rootDecider_,
            withdrawGlobalDecider_,
            withdrawLocalDecider_,
            singleWithdrawGlobalVerifier_,
            singleWithdrawLocalVerifier_
        );
    }

    /// @dev Internal initializer that wires storage pointers and seeds the transfer root history with the constant from the spec.
    /// forge-lint: disable-next-line(mixed-case-function)
    function __Verifier_init(
        address token_,
        uint32 hubEid_,
        address rootDecider_,
        address withdrawGlobalDecider_,
        address withdrawLocalDecider_,
        address singleWithdrawGlobalVerifier_,
        address singleWithdrawLocalVerifier_
    ) internal onlyInitializing {
        __Pausable_init();
        _token = token_;
        _hubEid = hubEid_;
        rootDecider = rootDecider_;
        withdrawGlobalDecider = withdrawGlobalDecider_;
        withdrawLocalDecider = withdrawLocalDecider_;
        singleWithdrawGlobalVerifier = singleWithdrawGlobalVerifier_;
        singleWithdrawLocalVerifier = singleWithdrawLocalVerifier_;

        emit VerifiersSet(
            rootDecider_,
            withdrawGlobalDecider_,
            withdrawLocalDecider_,
            singleWithdrawGlobalVerifier_,
            singleWithdrawLocalVerifier_
        );

        provedTransferRoots[0] = INITIAL_TRANSFER_ROOT;
        latestRelayedIndex = 0;
    }

    function _authorizeUpgrade(address) internal override onlyOwner {}

    /// -----------------------------------------------------------------------
    /// Transfer Root Functions
    /// -----------------------------------------------------------------------

    /// @notice Snapshots the latest `(index, hashChain)` tuple from zERC20 so Nova proofs can reference stable inputs.
    /// @dev Mirrors the first step of the private proof-of-burn lifecycle.
    /// @return index Reserved transfer index copied from zERC20.
    /// @return hashChain SHA-256 hash chain committed up to `index - 1`.
    function reserveHashChain() external returns (uint64 index, uint256 hashChain) {
        IzERC20 tokenContract = IzERC20(_token);
        uint64 index_ = uint64(tokenContract.index());
        uint256 hashChain_ = tokenContract.hashChain();
        reservedHashChains[index_] = hashChain_;
        latestReservedIndex = index_;
        emit HashChainReserved(index_, hashChain_);
        return (index_, hashChain_);
    }

    /// @notice Verifies a Nova proof for a transfer-root transition and records the resulting root by index.
    /// @dev Enforces consistency between (a) previously proved roots and (b) reserved hash chains, pausing on conflicts.
    /// @param proof Opaque calldata expected by `IRootDecider`, ABI-encoded as `uint256[32]`.
    function proveTransferRoot(bytes calldata proof) external whenNotPaused {
        uint256[32] memory proof_ = abi.decode(proof, (uint256[32]));
        uint64 oldIndex = uint64(proof_[1]);
        proof_[2]; // oldHashChain is unused
        uint256 oldRoot = proof_[3];
        uint64 newIndex = uint64(proof_[4]);
        uint256 newHashChain = proof_[5];
        uint256 newRoot = proof_[6];
        require(IRootDecider(rootDecider).verifyOpaqueNovaProof(proof_), InvalidProof());
        require(oldRoot != 0, OldRootZero(oldIndex));
        uint256 expectedOldRoot = provedTransferRoots[uint64(oldIndex)];
        require(expectedOldRoot == oldRoot, OldRootMismatch(oldIndex, expectedOldRoot, oldRoot));

        uint256 expectedHashChain = reservedHashChains[newIndex];
        require(expectedHashChain != 0, ReserveHashChainNotFound(newIndex));
        require(expectedHashChain == newHashChain, NewHashChainMismatch(newIndex, expectedHashChain, newHashChain));
        uint256 existingRoot = provedTransferRoots[newIndex];
        if (existingRoot != 0 && existingRoot != newRoot) {
            // non-determistic proof results - trigger emergency
            _pause();
            emit EmergencyTriggered(newIndex, existingRoot, newRoot);
            return;
        }
        provedTransferRoots[newIndex] = newRoot;
        if (newIndex > latestProvedIndex) {
            latestProvedIndex = newIndex;
        }
        emit TransferRootProved(newIndex, newRoot);
    }

    /// -----------------------------------------------------------------------
    /// Teleport Functions
    /// -----------------------------------------------------------------------

    /// @notice Executes the multi-note Nova teleport flow, minting the delta on zERC20.
    /// @dev Proof enforces that `totalValue` strictly increases per recipient hash; `isGlobal` selects local/global root arrays.
    /// @param isGlobal Whether the proof references Hub-derived global roots.
    /// @param rootHint Index into either `provedTransferRoots` or `globalTransferRoots`.
    /// @param gr GeneralRecipient struct encoding chain id, recipient, tweak, and version byte.
    /// @param proof ABI-encoded Nova proof blob consumed by `IWithdrawDecider`.
    function teleport(
        bool isGlobal,
        uint64 rootHint,
        GeneralRecipientLib.GeneralRecipient calldata gr,
        bytes calldata proof
    ) external whenNotPaused {
        // decode and verify proof
        uint256[34] memory proof_ = abi.decode(proof, (uint256[34]));
        uint256 transferRoot = proof_[1];
        uint256 recipient = proof_[2];
        require(proof_[3] == 0, InvalidInitialLastLeafIndex(proof_[3]));
        require(proof_[4] == 0, InvalidInitialTotalValue(proof_[4]));
        require(proof_[5] == transferRoot, FinalTransferRootMismatch(proof_[5], transferRoot));
        require(proof_[6] == recipient, FinalRecipientMismatch(proof_[6], recipient));
        proof_[7]; // lastLeafIndex is unused
        uint256 totalValue = proof_[8];
        address withdrawDecider = isGlobal ? withdrawGlobalDecider : withdrawLocalDecider;
        require(IWithdrawDecider(withdrawDecider).verifyOpaqueNovaProof(proof_), InvalidProof());

        _teleport(isGlobal, rootHint, transferRoot, recipient, gr, totalValue);
    }

    /// @notice Executes the Groth16 teleport flow for lightweight single withdrawals.
    /// @dev Shares the same recipient/root validation pipeline as `teleport` but operates on Groth16 proofs.
    /// @param isGlobal Whether the proof references Hub-derived global roots.
    /// @param rootHint Index into either `provedTransferRoots` or `globalTransferRoots`.
    /// @param gr GeneralRecipient struct encoding chain id, recipient, tweak, and version byte.
    /// @param proof ABI-encoded Groth16 proof blob consumed by `IWithdrawVerifier`.
    function singleTeleport(
        bool isGlobal,
        uint64 rootHint,
        GeneralRecipientLib.GeneralRecipient calldata gr,
        bytes calldata proof
    ) external whenNotPaused {
        // decode and verify proof
        (uint256[2] memory pA, uint256[2][2] memory pB, uint256[2] memory pC, uint256[3] memory pubSignals) =
            abi.decode(proof, (uint256[2], uint256[2][2], uint256[2], uint256[3]));
        uint256 transferRoot = pubSignals[0];
        uint256 recipient = pubSignals[1];
        uint256 value = pubSignals[2];
        address singleWithdrawVerifier = isGlobal ? singleWithdrawGlobalVerifier : singleWithdrawLocalVerifier;
        require(IWithdrawVerifier(singleWithdrawVerifier).verifyProof(pA, pB, pC, pubSignals), InvalidProof());

        _teleport(isGlobal, rootHint, transferRoot, recipient, gr, value);
    }

    /// @dev Shared logic for Nova and Groth16 teleports:
    ///      - Confirms the claimed root matches the hinted slot (local or global)
    ///      - Recomputes the recipient hash and chain id binding
    ///      - Mints only the delta above `totalTeleported[recipient]`.
    function _teleport(
        bool isGlobal,
        uint64 rootHint,
        uint256 transferRoot,
        uint256 recipient,
        GeneralRecipientLib.GeneralRecipient memory gr,
        uint256 value
    ) internal {
        // verify root
        uint256 expectedRoot = isGlobal ? globalTransferRoots[rootHint] : provedTransferRoots[rootHint];
        require(expectedRoot != 0, ExpectedRootZero(rootHint));
        require(expectedRoot == transferRoot, TransferRootMismatch(expectedRoot, transferRoot));

        // verify recipient
        uint256 expectedRecipient = gr.hash();
        require(recipient == expectedRecipient, RecipientMismatch(expectedRecipient, recipient));
        uint64 localChainId = uint64(block.chainid);
        require(gr.chainId == localChainId, InvalidRecipientChainId(gr.chainId, localChainId));

        uint256 currentTotal = totalTeleported[recipient];
        require(value > currentTotal, NothingToWithdraw(currentTotal, value));
        uint256 diff = value - currentTotal;
        totalTeleported[recipient] += diff;
        address recipientAddr = address(uint160(uint256(gr.recipient)));
        IzERC20(_token).teleport(recipientAddr, diff);
        emit Teleport(recipientAddr, diff);
    }

    /// -----------------------------------------------------------------------
    /// Relay Functions
    /// -----------------------------------------------------------------------

    /// @notice Sends the latest proved transfer root to the Hub over LayerZero so it can join the global aggregation tree.
    /// @dev Requires `latestProvedIndex` to have a non-zero root; excess msg.value is kept as the LZ native fee.
    /// @param options LayerZero execution parameters forwarded to `_lzSend`.
    function relayTransferRoot(bytes calldata options)
        external
        payable
        whenNotPaused
        returns (MessagingReceipt memory receipt)
    {
        uint64 index = latestProvedIndex;
        uint256 root = provedTransferRoots[index];
        if (root == 0) revert NoProvedRoot();

        bytes memory payload = abi.encode(root, index);
        MessagingFee memory quotedFee = _quote(_hubEid, payload, options, false);
        if (msg.value < quotedFee.nativeFee) {
            revert InsufficientMsgValue(quotedFee.nativeFee, msg.value);
        }

        MessagingFee memory fee = MessagingFee({nativeFee: msg.value, lzTokenFee: quotedFee.lzTokenFee});
        receipt = _lzSend(_hubEid, payload, options, fee, msg.sender);
        emit TransferRootRelayed(index, root, abi.encodePacked(receipt.guid));

        latestRelayedIndex = index;
    }

    /// @notice Quotes the native fee required to relay a TransferRoot payload to the Hub.
    /// @param options LayerZero execution parameters mirrored from `relayTransferRoot`.
    function quoteRelay(bytes calldata options) external view returns (MessagingFee memory fee) {
        bytes memory payload = abi.encode(uint256(0), uint64(0));
        return _quote(_hubEid, payload, options, false);
    }

    /// @notice Returns `true` when every proved root has been relayed to the Hub (`latestProvedIndex == latestRelayedIndex`).
    function isUpToDate() public view returns (bool) {
        return latestProvedIndex == latestRelayedIndex;
    }

    /// -----------------------------------------------------------------------
    /// LayerZero Receiver
    /// -----------------------------------------------------------------------

    /// @dev Accepts `(globalRoot, aggSeq)` payloads from the Hub, ignoring duplicates while tracking `latestAggSeq`.
    function _lzReceive(Origin calldata origin, bytes32, bytes calldata payload, address, bytes calldata)
        internal
        override
    {
        require(origin.srcEid == _hubEid, InvalidHubSource(origin.srcEid));

        (uint256 globalRoot, uint64 aggSeq_) = abi.decode(payload, (uint256, uint64));
        if (globalTransferRoots[aggSeq_] == 0) {
            globalTransferRoots[aggSeq_] = globalRoot;
            emit GlobalRootSaved(aggSeq_, globalRoot);
        }

        if (aggSeq_ > latestAggSeq) {
            latestAggSeq = aggSeq_;
        }
    }

    /// -----------------------------------------------------------------------
    /// Admin Functions
    /// -----------------------------------------------------------------------

    /// @notice Allows the owner to proactively pause the contract outside of proof conflicts.
    function activateEmergency() external onlyOwner {
        _pause();
    }

    /// @notice Clears the emergency pause that is triggered when conflicting transfer roots are observed.
    function deactivateEmergency() external onlyOwner {
        _unpause();
        emit DeactivateEmergency();
    }

    /// @notice Atomically rotates all decider/verifier addresses to keep proofs aligned with the latest deployments.
    /// @param newRootDecider Nova verifier for transfer roots.
    /// @param newWithdrawGlobalDecider Nova verifier for global teleports.
    /// @param newWithdrawLocalDecider Nova verifier for local teleports.
    /// @param newSingleWithdrawGlobalVerifier Groth16 verifier for global single teleports.
    /// @param newSingleWithdrawLocalVerifier Groth16 verifier for local single teleports.
    function setVerifiers(
        address newRootDecider,
        address newWithdrawGlobalDecider,
        address newWithdrawLocalDecider,
        address newSingleWithdrawGlobalVerifier,
        address newSingleWithdrawLocalVerifier
    ) external onlyOwner {
        if (
            newRootDecider == address(0) || newWithdrawGlobalDecider == address(0)
                || newWithdrawLocalDecider == address(0) || newSingleWithdrawGlobalVerifier == address(0)
                || newSingleWithdrawLocalVerifier == address(0)
        ) {
            revert ZeroAddress();
        }
        rootDecider = newRootDecider;
        withdrawGlobalDecider = newWithdrawGlobalDecider;
        withdrawLocalDecider = newWithdrawLocalDecider;
        singleWithdrawGlobalVerifier = newSingleWithdrawGlobalVerifier;
        singleWithdrawLocalVerifier = newSingleWithdrawLocalVerifier;
        emit VerifiersSet(
            rootDecider,
            withdrawGlobalDecider,
            withdrawLocalDecider,
            singleWithdrawGlobalVerifier,
            singleWithdrawLocalVerifier
        );
    }
}
