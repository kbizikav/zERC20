// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.33;

import {PausableUpgradeable} from "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import {ReentrancyGuardTransient} from "@openzeppelin/contracts/utils/ReentrancyGuardTransient.sol";
import {
    MessagingFee,
    MessagingReceipt
} from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/ILayerZeroEndpointV2.sol";
import {Origin} from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/ILayerZeroReceiver.sol";
import {IzERC20} from "./interfaces/IzERC20.sol";
import {IWithdrawDecider} from "./interfaces/IWithdrawDecider.sol";
import {IRootDecider} from "./interfaces/IRootDecider.sol";
import {IWithdrawVerifier} from "./interfaces/IVerifier.sol";
import {GeneralRecipientLib} from "./utils/GeneralRecipientLib.sol";
import {OAppUpgradeable} from "@layerzerolabs/oapp-evm-upgradeable/contracts/oapp/OAppUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {EIP712Upgradeable} from "@openzeppelin/contracts-upgradeable/utils/cryptography/EIP712Upgradeable.sol";
import {SignatureChecker} from "@openzeppelin/contracts/utils/cryptography/SignatureChecker.sol";

/**
 * @title Verifier
 * @notice LayerZero OApp that enforces the Nova / Groth16 teleport flows,
 *         acting as the bridge between on-chain hash-chain checkpoints, cross-chain aggregation roots, and zERC20 mints.
 * @dev Tracks reserved hash chains, proved transfer roots, global aggregation roots, and cumulative teleported totals.
 */
contract Verifier is
    OAppUpgradeable,
    PausableUpgradeable,
    ReentrancyGuardTransient,
    UUPSUpgradeable,
    EIP712Upgradeable
{
    using GeneralRecipientLib for GeneralRecipientLib.GeneralRecipient;

    event HashChainReserved(uint64 indexed index, uint256 hashChain);
    event TransferRootProved(uint64 indexed index, uint256 root);
    event TransferRootRelayed(uint64 indexed index, uint256 root, bytes lzMsgId);
    event GlobalRootSaved(uint64 indexed aggSeq, uint256 root);
    event EmergencyTriggered(uint64 indexed index, uint256 root1, uint256 root2);
    event ActivateEmergency();
    event DeactivateEmergency();
    event Teleport(
        address indexed to,
        uint256 value,
        bool isGlobal,
        uint64 rootHint,
        uint256 transferRoot,
        GeneralRecipientLib.GeneralRecipient gr
    );
    event TeleportWithRelayerFee(
        address indexed to,
        address indexed relayer,
        uint256 recipientValue,
        uint256 relayerFee,
        bool isGlobal,
        uint64 rootHint,
        uint256 transferRoot,
        GeneralRecipientLib.GeneralRecipient gr
    );
    // slither-disable-next-line unindexed-event
    event VerifiersSet(
        address rootDecider,
        address withdrawGlobalDecider,
        address withdrawLocalDecider,
        address singleWithdrawGlobalVerifier,
        address singleWithdrawLocalVerifier
    );

    error UnauthorizedProver(address caller);
    event ProverAllowed(address indexed prover);
    event ProverRemoved(address indexed prover);

    error RelayerFeeExceedsMax(uint256 relayerFee, uint256 maxFee);
    error RelayerFeeExceedsDiff(uint256 relayerFee, uint256 diff);
    error RelayerFeeAuthorizationExpired(uint64 deadline, uint256 currentTimestamp);
    error InvalidRelayerFeeSignature();

    error InvalidProof();
    error NoProvedRoot();
    error ZeroAddress();
    error InvalidHubSource(uint32 srcEid);
    error ZeroToken();
    error OldRootZero(uint64 index);
    error OldRootMismatch(uint64 index, uint256 expected, uint256 actual);
    error ReserveHashChainNotFound(uint64 index);
    error ZeroHashChain(uint64 index);
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
    error EndpointMismatch(address expected, address actual);
    error ZeroRefundAddress();

    bytes32 internal constant RELAYER_FEE_TYPEHASH =
        keccak256("RelayerFeeAuthorization(uint256 recipientHash,uint256 totalValue,uint256 maxFee,uint64 deadline)");

    /// @notice Parameters for relayer fee authorization, bundled to avoid stack-too-deep.
    /// @param relayerFee Actual fee claimed by the relayer.
    /// @param maxFee Maximum fee the recipient authorized via signature.
    /// @param deadline Timestamp after which the authorization expires.
    /// @param signature EIP-712 signature from the recipient.
    struct RelayerFeeAuthorization {
        uint256 relayerFee;
        uint256 maxFee;
        uint64 deadline;
        bytes signature;
    }

    // Root of an empty IncrementalMerkleTree at TRANSFER_TREE_HEIGHT (see zkp test).
    uint256 private constant INITIAL_TRANSFER_ROOT =
        8687547638004116013653730449839507042090717944911454416140763808366589487233;

    // ERC-7201 slot for namespace "zerc20.storage.verifier".
    bytes32 internal constant VERIFIER_STORAGE_SLOT =
        0xdc881cb05e4e981d28f2c49ec9604b5a0676bcc75afb8e4a2cd07140c6b7ae00;

    /// @custom:storage-location erc7201:zerc20.storage.verifier
    struct VerifierStorage {
        address token;
        uint32 hubEid;
        address rootDecider;
        address withdrawGlobalDecider;
        address withdrawLocalDecider;
        address singleWithdrawGlobalVerifier;
        address singleWithdrawLocalVerifier;
        uint64 latestReservedIndex;
        uint64 latestProvedIndex;
        uint64 latestAggSeq;
        uint64 latestRelayedIndex;
        mapping(uint64 => uint256) reservedHashChains;
        mapping(uint64 => uint256) provedTransferRoots;
        mapping(uint64 => uint256) globalTransferRoots;
        mapping(uint256 => uint256) totalTeleported;
    }

    // ERC-7201 slot for namespace "zerc20.storage.allowedProvers".
    // Isolated from VerifierStorage so the feature can be removed without layout impact.
    bytes32 private constant ALLOWED_PROVERS_SLOT = 0x7df33cd7bf86990bd72fbc637ced290e49788cd74afe06d17e0fa683b8fe4600;

    /// @custom:storage-location erc7201:zerc20.storage.allowedProvers
    struct AllowedProversStorage {
        mapping(address => bool) allowedProvers;
    }

    function _getVerifierStorage() private pure returns (VerifierStorage storage $) {
        bytes32 slot = VERIFIER_STORAGE_SLOT;
        // solhint-disable-next-line no-inline-assembly
        assembly {
            $.slot := slot
        }
    }

    function _getAllowedProversStorage() private pure returns (AllowedProversStorage storage $) {
        bytes32 slot = ALLOWED_PROVERS_SLOT;
        // solhint-disable-next-line no-inline-assembly
        assembly {
            $.slot := slot
        }
    }

    /// -----------------------------------------------------------------------
    /// View Functions
    /// -----------------------------------------------------------------------

    function token() public view returns (address) {
        return _getVerifierStorage().token;
    }

    function hubEid() public view returns (uint32) {
        return _getVerifierStorage().hubEid;
    }

    function rootDecider() public view returns (address) {
        return _getVerifierStorage().rootDecider;
    }

    function withdrawGlobalDecider() public view returns (address) {
        return _getVerifierStorage().withdrawGlobalDecider;
    }

    function withdrawLocalDecider() public view returns (address) {
        return _getVerifierStorage().withdrawLocalDecider;
    }

    function singleWithdrawGlobalVerifier() public view returns (address) {
        return _getVerifierStorage().singleWithdrawGlobalVerifier;
    }

    function singleWithdrawLocalVerifier() public view returns (address) {
        return _getVerifierStorage().singleWithdrawLocalVerifier;
    }

    function latestReservedIndex() public view returns (uint64) {
        return _getVerifierStorage().latestReservedIndex;
    }

    function latestProvedIndex() public view returns (uint64) {
        return _getVerifierStorage().latestProvedIndex;
    }

    function latestAggSeq() public view returns (uint64) {
        return _getVerifierStorage().latestAggSeq;
    }

    function latestRelayedIndex() public view returns (uint64) {
        return _getVerifierStorage().latestRelayedIndex;
    }

    function reservedHashChains(uint64 index) public view returns (uint256) {
        return _getVerifierStorage().reservedHashChains[index];
    }

    function provedTransferRoots(uint64 index) public view returns (uint256) {
        return _getVerifierStorage().provedTransferRoots[index];
    }

    function globalTransferRoots(uint64 index) public view returns (uint256) {
        return _getVerifierStorage().globalTransferRoots[index];
    }

    function totalTeleported(uint256 recipient) public view returns (uint256) {
        return _getVerifierStorage().totalTeleported[recipient];
    }

    function isAllowedProver(address prover) public view returns (bool) {
        return _getAllowedProversStorage().allowedProvers[prover];
    }

    constructor(address endpoint) OAppUpgradeable(endpoint) {
        require(endpoint != address(0), InvalidEndpointCall());
        _disableInitializers();
    }

    /// @notice Initializes the verifier with the zERC20 token, Hub endpoint, LayerZero delegate, and initial deciders.
    /// @param token_ zERC20 token whose hash chain is reserved/minted against.
    /// @param hubEid_ LayerZero endpoint ID of the Hub contract.
    /// @param delegate Address that MUST be the contract owner; it is set as both Ownable owner and LayerZero delegate.
    /// @param rootDecider_ Nova verifier for transfer-root transitions.
    /// @param withdrawGlobalDecider_ Nova verifier for global teleport proofs.
    /// @param withdrawLocalDecider_ Nova verifier for local teleport proofs.
    /// @param singleWithdrawGlobalVerifier_ Groth16 verifier for global single teleports.
    /// @param singleWithdrawLocalVerifier_ Groth16 verifier for local single teleports.
    function initialize(
        address token_,
        uint32 hubEid_,
        address delegate,
        address rootDecider_,
        address withdrawGlobalDecider_,
        address withdrawLocalDecider_,
        address singleWithdrawGlobalVerifier_,
        address singleWithdrawLocalVerifier_
    ) external initializer {
        require(token_ != address(0), ZeroToken());
        require(delegate != address(0), ZeroAddress());
        require(
            rootDecider_ != address(0) && withdrawGlobalDecider_ != address(0) && withdrawLocalDecider_ != address(0)
                && singleWithdrawGlobalVerifier_ != address(0) && singleWithdrawLocalVerifier_ != address(0),
            ZeroAddress()
        );

        __Ownable_init(delegate);
        __OApp_init(delegate);
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
        VerifierStorage storage $ = _getVerifierStorage();
        $.token = token_;
        $.hubEid = hubEid_;
        $.rootDecider = rootDecider_;
        $.withdrawGlobalDecider = withdrawGlobalDecider_;
        $.withdrawLocalDecider = withdrawLocalDecider_;
        $.singleWithdrawGlobalVerifier = singleWithdrawGlobalVerifier_;
        $.singleWithdrawLocalVerifier = singleWithdrawLocalVerifier_;

        emit VerifiersSet(
            rootDecider_,
            withdrawGlobalDecider_,
            withdrawLocalDecider_,
            singleWithdrawGlobalVerifier_,
            singleWithdrawLocalVerifier_
        );

        $.provedTransferRoots[0] = INITIAL_TRANSFER_ROOT;
        $.latestRelayedIndex = 0;
    }

    /// @notice Re-initializer that activates EIP-712 typed-data signing for relayer fee authorizations.
    /// @param name_ EIP-712 domain name.
    /// @param version_ EIP-712 domain version.
    function initializeV2(string calldata name_, string calldata version_) external reinitializer(2) onlyOwner {
        __EIP712_init(name_, version_);
    }

    function _authorizeUpgrade(address newImplementation) internal view override onlyOwner {
        address expected = address(endpoint);
        address actual = address(Verifier(newImplementation).endpoint());
        require(actual == expected, EndpointMismatch(expected, actual));
    }

    /// -----------------------------------------------------------------------
    /// Transfer Root Functions
    /// -----------------------------------------------------------------------

    /// @notice Snapshots the latest `(index, hashChain)` tuple from zERC20 so Nova proofs can reference stable inputs.
    /// @dev Mirrors the first step of the private proof-of-burn lifecycle.
    /// @dev This function is intentionally NOT gated by `whenNotPaused`.
    ///      Emergency pause is meant to halt ZKP verification and any ZKP-derived actions (prove/teleport/relay),
    ///      while still allowing non-ZKP flows (e.g., transfers/OFT bridge activity) to continue and reservations
    ///      to be prepared for later proof submission.
    /// @return index Reserved transfer index copied from zERC20.
    /// @return hashChain SHA-256 hash chain committed up to `index - 1`.
    function reserveHashChain() external returns (uint64 index, uint256 hashChain) {
        VerifierStorage storage $ = _getVerifierStorage();
        IzERC20 tokenContract = IzERC20($.token);
        uint64 index_ = uint64(tokenContract.index());
        uint256 hashChain_ = tokenContract.hashChain();
        require(hashChain_ != 0, ZeroHashChain(index_));
        $.reservedHashChains[index_] = hashChain_;
        $.latestReservedIndex = index_;
        emit HashChainReserved(index_, hashChain_);
        return (index_, hashChain_);
    }

    modifier onlyAllowedProver() {
        require(_getAllowedProversStorage().allowedProvers[msg.sender], UnauthorizedProver(msg.sender));
        _;
    }

    /// @notice Verifies a Nova proof for a transfer-root transition and records the resulting root by index.
    /// @dev Enforces consistency between (a) previously proved roots and (b) reserved hash chains, pausing on conflicts.
    /// @param proof Opaque calldata expected by `IRootDecider`, ABI-encoded as `uint256[32]`.
    function proveTransferRoot(bytes calldata proof) external whenNotPaused onlyAllowedProver {
        uint256[32] memory proof_ = abi.decode(proof, (uint256[32]));
        uint64 oldIndex = uint64(proof_[1]);
        // proof_[2] is oldHashChain, intentionally unused in on-chain verification
        uint256 oldRoot = proof_[3];
        uint64 newIndex = uint64(proof_[4]);
        uint256 newHashChain = proof_[5];
        uint256 newRoot = proof_[6];
        VerifierStorage storage $ = _getVerifierStorage();
        require(IRootDecider($.rootDecider).verifyOpaqueNovaProof(proof_), InvalidProof());
        require(oldRoot != 0, OldRootZero(oldIndex));
        uint256 expectedOldRoot = $.provedTransferRoots[uint64(oldIndex)];
        require(expectedOldRoot == oldRoot, OldRootMismatch(oldIndex, expectedOldRoot, oldRoot));

        uint256 expectedHashChain = $.reservedHashChains[newIndex];
        require(expectedHashChain != 0, ReserveHashChainNotFound(newIndex));
        require(expectedHashChain == newHashChain, NewHashChainMismatch(newIndex, expectedHashChain, newHashChain));
        uint256 existingRoot = $.provedTransferRoots[newIndex];
        if (existingRoot != 0 && existingRoot != newRoot) {
            // non-deterministic proof results - trigger emergency
            _pause();
            emit EmergencyTriggered(newIndex, existingRoot, newRoot);
            return;
        }
        $.provedTransferRoots[newIndex] = newRoot;
        if (newIndex > $.latestProvedIndex) {
            $.latestProvedIndex = newIndex;
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
    ) external whenNotPaused nonReentrant {
        (uint256 transferRoot, uint256 recipient, uint256 totalValue) = _verifyNovaProof(isGlobal, proof);
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
    ) external whenNotPaused nonReentrant {
        (uint256 transferRoot, uint256 recipient, uint256 totalValue) = _verifySingleProof(isGlobal, proof);
        _teleport(isGlobal, rootHint, transferRoot, recipient, gr, totalValue);
    }

    /// @notice Executes the multi-note Nova teleport flow with relayer fee distribution.
    /// @param isGlobal Whether the proof references Hub-derived global roots.
    /// @param rootHint Index into either `provedTransferRoots` or `globalTransferRoots`.
    /// @param gr GeneralRecipient struct encoding chain id, recipient, tweak, and version byte.
    /// @param proof ABI-encoded Nova proof blob consumed by `IWithdrawDecider`.
    /// @param feeAuth Relayer fee authorization parameters including signature.
    function teleport(
        bool isGlobal,
        uint64 rootHint,
        GeneralRecipientLib.GeneralRecipient calldata gr,
        bytes calldata proof,
        RelayerFeeAuthorization calldata feeAuth
    ) external whenNotPaused nonReentrant {
        (uint256 transferRoot, uint256 recipient, uint256 totalValue) = _verifyNovaProof(isGlobal, proof);
        _verifyRelayerFeeAuthorization(
            recipient,
            totalValue,
            feeAuth.relayerFee,
            feeAuth.maxFee,
            feeAuth.deadline,
            feeAuth.signature,
            _bytes32ToAddress(gr.recipient)
        );
        _teleportWithFee(isGlobal, rootHint, transferRoot, recipient, gr, totalValue, feeAuth.relayerFee);
    }

    /// @notice Executes the Groth16 teleport flow with relayer fee distribution.
    /// @param isGlobal Whether the proof references Hub-derived global roots.
    /// @param rootHint Index into either `provedTransferRoots` or `globalTransferRoots`.
    /// @param gr GeneralRecipient struct encoding chain id, recipient, tweak, and version byte.
    /// @param proof ABI-encoded Groth16 proof blob consumed by `IWithdrawVerifier`.
    /// @param feeAuth Relayer fee authorization parameters including signature.
    function singleTeleport(
        bool isGlobal,
        uint64 rootHint,
        GeneralRecipientLib.GeneralRecipient calldata gr,
        bytes calldata proof,
        RelayerFeeAuthorization calldata feeAuth
    ) external whenNotPaused nonReentrant {
        (uint256 transferRoot, uint256 recipient, uint256 totalValue) = _verifySingleProof(isGlobal, proof);
        _verifyRelayerFeeAuthorization(
            recipient,
            totalValue,
            feeAuth.relayerFee,
            feeAuth.maxFee,
            feeAuth.deadline,
            feeAuth.signature,
            _bytes32ToAddress(gr.recipient)
        );
        _teleportWithFee(isGlobal, rootHint, transferRoot, recipient, gr, totalValue, feeAuth.relayerFee);
    }

    /// @dev Decodes and verifies a Nova withdraw proof, returning extracted public signals.
    function _verifyNovaProof(bool isGlobal, bytes calldata proof)
        internal
        view
        returns (uint256 transferRoot, uint256 recipient, uint256 totalValue)
    {
        uint256[34] memory proof_ = abi.decode(proof, (uint256[34]));
        transferRoot = proof_[1];
        recipient = proof_[2];
        require(proof_[3] == 0, InvalidInitialLastLeafIndex(proof_[3]));
        require(proof_[4] == 0, InvalidInitialTotalValue(proof_[4]));
        require(proof_[5] == transferRoot, FinalTransferRootMismatch(proof_[5], transferRoot));
        require(proof_[6] == recipient, FinalRecipientMismatch(proof_[6], recipient));
        // lastLeafIndex is visible in calldata; provers should pad to the maximum index to avoid leakage.
        proof_[7];
        totalValue = proof_[8];
        VerifierStorage storage $ = _getVerifierStorage();
        address withdrawDecider = isGlobal ? $.withdrawGlobalDecider : $.withdrawLocalDecider;
        require(IWithdrawDecider(withdrawDecider).verifyOpaqueNovaProof(proof_), InvalidProof());
    }

    /// @dev Decodes and verifies a Groth16 single-withdraw proof, returning extracted public signals.
    function _verifySingleProof(bool isGlobal, bytes calldata proof)
        internal
        view
        returns (uint256 transferRoot, uint256 recipient, uint256 totalValue)
    {
        (uint256[2] memory pA, uint256[2][2] memory pB, uint256[2] memory pC, uint256[3] memory pubSignals) =
            abi.decode(proof, (uint256[2], uint256[2][2], uint256[2], uint256[3]));
        VerifierStorage storage $ = _getVerifierStorage();
        address singleWithdrawVerifier = isGlobal ? $.singleWithdrawGlobalVerifier : $.singleWithdrawLocalVerifier;
        require(IWithdrawVerifier(singleWithdrawVerifier).verifyProof(pA, pB, pC, pubSignals), InvalidProof());
        return (pubSignals[0], pubSignals[1], pubSignals[2]);
    }

    /// @dev Validates root, recipient, and chain id; updates `totalTeleported`; returns the mint delta.
    function _validateAndUpdateTeleport(
        bool isGlobal,
        uint64 rootHint,
        uint256 transferRoot,
        uint256 recipient,
        GeneralRecipientLib.GeneralRecipient memory gr,
        uint256 value
    ) internal returns (uint256 diff) {
        VerifierStorage storage $ = _getVerifierStorage();
        // verify root
        uint256 expectedRoot = isGlobal ? $.globalTransferRoots[rootHint] : $.provedTransferRoots[rootHint];
        require(expectedRoot != 0, ExpectedRootZero(rootHint));
        require(expectedRoot == transferRoot, TransferRootMismatch(expectedRoot, transferRoot));

        // verify recipient
        uint256 expectedRecipient = gr.hash();
        require(recipient == expectedRecipient, RecipientMismatch(expectedRecipient, recipient));
        uint64 localChainId = uint64(block.chainid);
        require(gr.chainId == localChainId, InvalidRecipientChainId(gr.chainId, localChainId));

        uint256 currentTotal = $.totalTeleported[recipient];
        require(value > currentTotal, NothingToWithdraw(currentTotal, value));
        diff = value - currentTotal;
        $.totalTeleported[recipient] = value;
    }

    /// @dev Shared logic for Nova and Groth16 teleports (no relayer fee).
    function _teleport(
        bool isGlobal,
        uint64 rootHint,
        uint256 transferRoot,
        uint256 recipient,
        GeneralRecipientLib.GeneralRecipient memory gr,
        uint256 value
    ) internal {
        uint256 diff = _validateAndUpdateTeleport(isGlobal, rootHint, transferRoot, recipient, gr, value);
        address recipientAddr = _bytes32ToAddress(gr.recipient);
        IzERC20(_getVerifierStorage().token).teleport(recipientAddr, diff);
        emit Teleport(recipientAddr, diff, isGlobal, rootHint, transferRoot, gr);
    }

    /// @dev Verifies an EIP-712 relayer fee authorization signature.
    /// @param recipientHash Hash of the GeneralRecipient struct.
    /// @param totalValue Cumulative teleport value (acts as natural nonce).
    /// @param relayerFee Actual fee the relayer is claiming.
    /// @param maxFee Maximum fee the signer authorized.
    /// @param deadline Timestamp after which the authorization expires.
    /// @param signature EIP-712 signature from the recipient (EOA or ERC-1271).
    /// @param signer Address of the recipient who signed the authorization.
    function _verifyRelayerFeeAuthorization(
        uint256 recipientHash,
        uint256 totalValue,
        uint256 relayerFee,
        uint256 maxFee,
        uint64 deadline,
        bytes calldata signature,
        address signer
    ) internal view {
        require(block.timestamp <= deadline, RelayerFeeAuthorizationExpired(deadline, block.timestamp));
        require(relayerFee <= maxFee, RelayerFeeExceedsMax(relayerFee, maxFee));
        bytes32 structHash = keccak256(abi.encode(RELAYER_FEE_TYPEHASH, recipientHash, totalValue, maxFee, deadline));
        bytes32 digest = _hashTypedDataV4(structHash);
        require(SignatureChecker.isValidSignatureNowCalldata(signer, digest, signature), InvalidRelayerFeeSignature());
    }

    /// @dev Shared logic for relayer-fee teleports: validates root/recipient, distributes fee, mints remainder.
    ///      Caller must have already verified the relayer fee authorization signature.
    function _teleportWithFee(
        bool isGlobal,
        uint64 rootHint,
        uint256 transferRoot,
        uint256 recipient,
        GeneralRecipientLib.GeneralRecipient memory gr,
        uint256 value,
        uint256 relayerFee
    ) internal {
        uint256 diff = _validateAndUpdateTeleport(isGlobal, rootHint, transferRoot, recipient, gr, value);
        require(relayerFee <= diff, RelayerFeeExceedsDiff(relayerFee, diff));
        {
            address recipientAddr = _bytes32ToAddress(gr.recipient);
            address tokenAddr = _getVerifierStorage().token;
            IzERC20(tokenAddr).teleport(recipientAddr, diff - relayerFee);
            if (relayerFee > 0) {
                IzERC20(tokenAddr).teleport(msg.sender, relayerFee);
            }
        }
        emit TeleportWithRelayerFee(
            _bytes32ToAddress(gr.recipient),
            msg.sender,
            diff - relayerFee,
            relayerFee,
            isGlobal,
            rootHint,
            transferRoot,
            gr
        );
    }

    function _bytes32ToAddress(bytes32 value) private pure returns (address) {
        return address(uint160(uint256(value)));
    }

    /// -----------------------------------------------------------------------
    /// Relay Functions
    /// -----------------------------------------------------------------------

    /// @notice Sends the latest proved transfer root to the Hub over LayerZero so it can join the global aggregation tree.
    /// @dev Requires `latestProvedIndex` to have a non-zero root; excess msg.value (if any) is refunded by the endpoint.
    /// @param options LayerZero execution parameters forwarded to `_lzSend`.
    function relayTransferRoot(bytes calldata options)
        external
        payable
        whenNotPaused
        returns (MessagingReceipt memory receipt)
    {
        receipt = _relayTransferRoot(options, msg.sender);
    }

    function relayTransferRoot(bytes calldata options, address refundAddress)
        external
        payable
        whenNotPaused
        returns (MessagingReceipt memory receipt)
    {
        receipt = _relayTransferRoot(options, refundAddress);
    }

    function _relayTransferRoot(bytes calldata options, address refundAddress)
        internal
        returns (MessagingReceipt memory receipt)
    {
        require(refundAddress != address(0), ZeroRefundAddress());
        VerifierStorage storage $ = _getVerifierStorage();
        uint64 index = $.latestProvedIndex;
        uint256 root = $.provedTransferRoots[index];
        require(root != 0, NoProvedRoot());

        bytes memory payload = abi.encode(root, index);
        MessagingFee memory quotedFee = _quote($.hubEid, payload, options, false);
        require(msg.value >= quotedFee.nativeFee, InsufficientMsgValue(quotedFee.nativeFee, msg.value));

        MessagingFee memory fee = MessagingFee({nativeFee: msg.value, lzTokenFee: quotedFee.lzTokenFee});
        receipt = _lzSend($.hubEid, payload, options, fee, refundAddress);
        emit TransferRootRelayed(index, root, abi.encodePacked(receipt.guid));

        $.latestRelayedIndex = index;
    }

    /// @notice Quotes the native fee required to relay a TransferRoot payload to the Hub.
    /// @param options LayerZero execution parameters mirrored from `relayTransferRoot`.
    function quoteRelay(bytes calldata options) external view returns (MessagingFee memory fee) {
        bytes memory payload = abi.encode(uint256(0), uint64(0));
        VerifierStorage storage $ = _getVerifierStorage();
        return _quote($.hubEid, payload, options, false);
    }

    /// @notice Returns `true` when every proved root has been relayed to the Hub (`latestProvedIndex == latestRelayedIndex`).
    function isUpToDate() public view returns (bool) {
        VerifierStorage storage $ = _getVerifierStorage();
        return $.latestProvedIndex == $.latestRelayedIndex;
    }

    /// -----------------------------------------------------------------------
    /// LayerZero Receiver
    /// -----------------------------------------------------------------------

    /// @dev Accepts `(globalRoot, aggSeq)` payloads from the Hub, ignoring duplicates while tracking `latestAggSeq`.
    function _lzReceive(Origin calldata origin, bytes32, bytes calldata payload, address, bytes calldata)
        internal
        override
    {
        VerifierStorage storage $ = _getVerifierStorage();
        require(origin.srcEid == $.hubEid, InvalidHubSource(origin.srcEid));

        (uint256 globalRoot, uint64 aggSeq_) = abi.decode(payload, (uint256, uint64));
        if ($.globalTransferRoots[aggSeq_] == 0) {
            $.globalTransferRoots[aggSeq_] = globalRoot;
            emit GlobalRootSaved(aggSeq_, globalRoot);
        }

        if (aggSeq_ > $.latestAggSeq) {
            $.latestAggSeq = aggSeq_;
        }
    }

    /// -----------------------------------------------------------------------
    /// Admin Functions
    /// -----------------------------------------------------------------------

    /// @notice Allows the owner to proactively pause the contract outside of proof conflicts.
    function activateEmergency() external onlyOwner {
        _pause();
        emit ActivateEmergency();
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
        require(
            newRootDecider != address(0) && newWithdrawGlobalDecider != address(0)
                && newWithdrawLocalDecider != address(0) && newSingleWithdrawGlobalVerifier != address(0)
                && newSingleWithdrawLocalVerifier != address(0),
            ZeroAddress()
        );
        VerifierStorage storage $ = _getVerifierStorage();
        $.rootDecider = newRootDecider;
        $.withdrawGlobalDecider = newWithdrawGlobalDecider;
        $.withdrawLocalDecider = newWithdrawLocalDecider;
        $.singleWithdrawGlobalVerifier = newSingleWithdrawGlobalVerifier;
        $.singleWithdrawLocalVerifier = newSingleWithdrawLocalVerifier;
        emit VerifiersSet(
            $.rootDecider,
            $.withdrawGlobalDecider,
            $.withdrawLocalDecider,
            $.singleWithdrawGlobalVerifier,
            $.singleWithdrawLocalVerifier
        );
    }

    /// @notice Grants `prover` permission to call `proveTransferRoot`.
    /// @param prover Address to allow.
    function setAllowedProver(address prover) external onlyOwner {
        require(prover != address(0), ZeroAddress());
        _getAllowedProversStorage().allowedProvers[prover] = true;
        emit ProverAllowed(prover);
    }

    /// @notice Revokes `prover` permission to call `proveTransferRoot`.
    /// @param prover Address to remove.
    function removeAllowedProver(address prover) external onlyOwner {
        require(prover != address(0), ZeroAddress());
        _getAllowedProversStorage().allowedProvers[prover] = false;
        emit ProverRemoved(prover);
    }
}
