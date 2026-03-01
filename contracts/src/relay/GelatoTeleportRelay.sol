// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Address} from "@openzeppelin/contracts/utils/Address.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {GelatoRelayContractsUtils} from "relay-context-contracts/utils/GelatoRelayContractsUtils.sol";
import {NATIVE_TOKEN} from "relay-context-contracts/constants/Tokens.sol";
import {Verifier} from "../Verifier.sol";
import {ILiquidityManager} from "../interfaces/ILiquidityManager.sol";
import {GeneralRecipientLib} from "../utils/GeneralRecipientLib.sol";

/// @title GelatoTeleportRelay
/// @notice Gelato Relay integration for gasless teleport via `callWithSyncFee`.
/// @dev Flow: Verifier.teleport → zERC20 relayerFee minted to this contract →
///      LiquidityManager.unwrap → underlying → Gelato fee payment.
///      Implements GelatoRelayContext-equivalent logic inline to avoid OZ version
///      incompatibility with relay-context-contracts' TokenUtils.
contract GelatoTeleportRelay is GelatoRelayContractsUtils, UUPSUpgradeable, OwnableUpgradeable {
    using SafeERC20 for IERC20;

    uint256 private constant _FEE_COLLECTOR_START = 72;
    uint256 private constant _FEE_TOKEN_START = 52;
    uint256 private constant _FEE_START = 32;

    Verifier public immutable VERIFIER;
    ILiquidityManager public immutable LIQUIDITY_MANAGER;
    IERC20 public immutable UNDERLYING_TOKEN;
    IERC20 public immutable ZERC20_TOKEN;

    error ZeroAddress();
    error OnlyGelatoRelay();
    error MaxFeeExceeded(uint256 fee, uint256 maxFee);
    error VerifierMismatch(address expected, address actual);
    error LiquidityManagerMismatch(address expected, address actual);

    modifier onlyGelatoRelay() {
        require(msg.sender == _gelatoRelay, OnlyGelatoRelay());
        _;
    }

    /// @notice Locks implementation contracts on deployment.
    constructor(address verifier_, address liquidityManager_) {
        require(verifier_ != address(0), ZeroAddress());
        require(liquidityManager_ != address(0), ZeroAddress());

        VERIFIER = Verifier(verifier_);
        LIQUIDITY_MANAGER = ILiquidityManager(liquidityManager_);
        UNDERLYING_TOKEN = LIQUIDITY_MANAGER.underlyingToken();
        ZERC20_TOKEN = IERC20(address(LIQUIDITY_MANAGER.zerc20()));
        _disableInitializers();
    }

    /// @notice Initializes the relay with its owner.
    /// @param initialOwner Account receiving ownership and upgrade authority.
    function initialize(address initialOwner) external initializer {
        require(initialOwner != address(0), ZeroAddress());
        __Ownable_init(initialOwner);
    }

    /// @dev Validates that the new implementation uses the same immutable dependencies.
    function _authorizeUpgrade(address newImplementation) internal view override onlyOwner {
        GelatoTeleportRelay candidate = GelatoTeleportRelay(payable(newImplementation));
        address expectedVerifier = address(VERIFIER);
        address actualVerifier = address(candidate.VERIFIER());
        require(actualVerifier == expectedVerifier, VerifierMismatch(expectedVerifier, actualVerifier));
        address expectedManager = address(LIQUIDITY_MANAGER);
        address actualManager = address(candidate.LIQUIDITY_MANAGER());
        require(actualManager == expectedManager, LiquidityManagerMismatch(expectedManager, actualManager));
    }

    /// @notice Relays a multi-note Nova teleport via Gelato.
    /// @param isGlobal Whether the proof references Hub-derived global roots.
    /// @param rootHint Index into proved/global transfer roots.
    /// @param gr GeneralRecipient struct encoding chain id, recipient, tweak.
    /// @param proof ABI-encoded Nova proof blob.
    /// @param feeAuth Relayer fee authorization parameters.
    /// @param maxGelatoFee Maximum acceptable Gelato fee in underlying token units.
    function relayTeleport(
        bool isGlobal,
        uint64 rootHint,
        GeneralRecipientLib.GeneralRecipient calldata gr,
        bytes calldata proof,
        Verifier.RelayerFeeAuthorization calldata feeAuth,
        uint256 maxGelatoFee
    ) external onlyGelatoRelay {
        VERIFIER.teleport(isGlobal, rootHint, gr, proof, feeAuth);
        _unwrapAndPayGelato(feeAuth.relayerFee, maxGelatoFee);
    }

    /// @notice Relays a Groth16 single teleport via Gelato.
    /// @param isGlobal Whether the proof references Hub-derived global roots.
    /// @param rootHint Index into proved/global transfer roots.
    /// @param gr GeneralRecipient struct encoding chain id, recipient, tweak.
    /// @param proof ABI-encoded Groth16 proof blob.
    /// @param feeAuth Relayer fee authorization parameters.
    /// @param maxGelatoFee Maximum acceptable Gelato fee in underlying token units.
    function relaySingleTeleport(
        bool isGlobal,
        uint64 rootHint,
        GeneralRecipientLib.GeneralRecipient calldata gr,
        bytes calldata proof,
        Verifier.RelayerFeeAuthorization calldata feeAuth,
        uint256 maxGelatoFee
    ) external onlyGelatoRelay {
        VERIFIER.singleTeleport(isGlobal, rootHint, gr, proof, feeAuth);
        _unwrapAndPayGelato(feeAuth.relayerFee, maxGelatoFee);
    }

    /// @notice Withdraws surplus ERC20 tokens to `to`.
    /// @param token Token to withdraw.
    /// @param to Recipient address.
    /// @param amount Amount to withdraw.
    function withdrawSurplus(address token, address to, uint256 amount) external onlyOwner {
        IERC20(token).safeTransfer(to, amount);
    }

    /// @notice Withdraws surplus native tokens to `to`.
    /// @param to Recipient address.
    /// @param amount Amount to withdraw.
    function withdrawSurplusNative(address payable to, uint256 amount) external onlyOwner {
        Address.sendValue(to, amount);
    }

    /// @dev Unwraps zERC20 to underlying via LiquidityManager, then pays Gelato.
    function _unwrapAndPayGelato(uint256 relayerFee, uint256 maxGelatoFee) internal {
        if (relayerFee > 0) {
            // slither-disable-next-line unused-return
            LIQUIDITY_MANAGER.unwrap(relayerFee, address(this));
        }
        _transferRelayFeeCapped(maxGelatoFee);
    }

    /// @dev Extracts Gelato fee context from appended calldata and transfers the fee.
    ///      Equivalent to GelatoRelayContext._transferRelayFeeCapped but uses OZ 5.x SafeERC20.
    function _transferRelayFeeCapped(uint256 maxFee) internal {
        uint256 fee = _getFee();
        require(fee <= maxFee, MaxFeeExceeded(fee, maxFee));
        address feeToken = _getFeeToken();
        address feeCollector = _getFeeCollector();
        if (feeToken == NATIVE_TOKEN) {
            Address.sendValue(payable(feeCollector), fee);
        } else {
            IERC20(feeToken).safeTransfer(feeCollector, fee);
        }
    }

    function _getFeeCollector() internal pure returns (address feeCollector) {
        // solhint-disable-next-line no-inline-assembly
        assembly {
            feeCollector := shr(96, calldataload(sub(calldatasize(), _FEE_COLLECTOR_START)))
        }
    }

    function _getFeeToken() internal pure returns (address feeToken) {
        // solhint-disable-next-line no-inline-assembly
        assembly {
            feeToken := shr(96, calldataload(sub(calldatasize(), _FEE_TOKEN_START)))
        }
    }

    function _getFee() internal pure returns (uint256 fee) {
        // solhint-disable-next-line no-inline-assembly
        assembly {
            fee := calldataload(sub(calldatasize(), _FEE_START))
        }
    }

    /// @dev Accept native underlying (ETH/BNB) from LiquidityManager.unwrap().
    receive() external payable {}
}
