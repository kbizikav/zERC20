// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {IzERC20} from "./interfaces/IzERC20.sol";
import {ShaHashChainLib} from "./utils/ShaHashChainLib.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {OFTUpgradeable} from "./utils/layerzero/oft/OFTUpgradeable.sol";
import {SlotDerivation} from "@openzeppelin/contracts/utils/SlotDerivation.sol";

/// @title zERC20
/// @notice Upgradeable ERC20 token that feeds the zk circuits by enforcing 248-bit transfer values,
///         hashing `(to, value)` pairs into a SHA-256 chain, and gating mint/burn roles for the Verifier and Minter flows.
///         Also implements the LayerZero V2 OFT interface for omnichain transfers.
contract zERC20 is OFTUpgradeable, UUPSUpgradeable, IzERC20 {
    using SlotDerivation for string;

    /// @custom:storage-location erc7201:zerc20.storage.zerc20
    struct ZERC20Storage {
        uint256 hashChain;
        uint256 index;
        address verifier;
        address minter;
        uint8 decimals;
    }

    function _getZERC20Storage() private pure returns (ZERC20Storage storage $) {
        bytes32 slot = SlotDerivation.erc7201Slot("zerc20.storage.zerc20");
        assembly {
            $.slot := slot
        }
    }

    /// @notice Emitted when the verifier address changes.
    event VerifierUpdated(address indexed newVerifier);
    /// @notice Emitted when the minter address changes.
    event MinterUpdated(address indexed newMinter);

    /// @notice Reverts when a caller other than the verifier invokes a verifier-only entrypoint.
    error OnlyVerifier();
    /// @notice Reverts when a caller other than the minter invokes a minter-only entrypoint.
    error OnlyMinter();
    /// @notice Reverts when an operation receives the zero address.
    error ZeroAddress();
    /// @notice Reverts when a value exceeds the supported 248-bit range.
    error ValueTooLarge();
    /// @notice Reverts when the requested decimals are incompatible with shared decimals.
    error InvalidDecimals();

    /// @notice Locks implementation contracts on deployment.
    constructor() {
        _disableInitializers();
    }

    /// @notice Initializes token metadata and ownership.
    /// @param name_ ERC20 name.
    /// @param symbol_ ERC20 symbol.
    /// @param initialOwner Account receiving ownership, LayerZero delegate permissions, and upgrade authority.
    /// @param endpoint LayerZero endpoint used for OFT messaging.
    /// @param decimals_ Token decimals; must be >= shared decimals.
    function initialize(
        string memory name_,
        string memory symbol_,
        address initialOwner,
        address endpoint,
        uint8 decimals_
    ) external initializer {
        if (initialOwner == address(0) || endpoint == address(0)) revert ZeroAddress();
        if (decimals_ < sharedDecimals()) revert InvalidDecimals();

        _getZERC20Storage().decimals = decimals_;
        __OFT_init(name_, symbol_, endpoint, initialOwner);
        __UUPSUpgradeable_init();
    }

    /// @dev Restricts upgrade authorization to the owner.
    function _authorizeUpgrade(address) internal override onlyOwner {}

    /// @notice Hash chain committing every transfer's destination and value pair.
    function hashChain() public view returns (uint256) {
        return _getZERC20Storage().hashChain;
    }

    /// @notice Index of the next transfer, matching the off-chain Merkle tree leaf position.
    function index() public view returns (uint256) {
        return _getZERC20Storage().index;
    }

    /// @notice Address allowed to call verifier-only functions such as teleport.
    function verifier() public view returns (address) {
        return _getZERC20Storage().verifier;
    }

    /// @notice Returns the token decimals.
    function decimals() public view override returns (uint8) {
        return _getZERC20Storage().decimals;
    }

    /// @notice Address allowed to mint and burn under the minter role.
    function minter() public view returns (address) {
        return _getZERC20Storage().minter;
    }

    /// @inheritdoc IzERC20
    /// @dev Called exclusively by the Verifier once a teleport proof succeeds.
    /// @param to Recipient mandated by the zero-knowledge proof (already hashed into the public inputs).
    /// @param value Mint amount corresponding to the delta proven in Verifier.teleport.
    function teleport(address to, uint256 value) external {
        if (msg.sender != verifier()) revert OnlyVerifier();
        _mint(to, value);
        emit Teleport(to, value);
    }

    /// @dev Commits every transfer (including mint/burn) to the 248-bit SHA-256 hash chain described in the spec.
    ///      Reverts if the amount exceeds the BN254-friendly bound so that the proof circuits remain well-defined.
    function _afterTokenTransfer(address from, address to, uint256 value) internal override {
        if (value > type(uint248).max) revert ValueTooLarge();
        ZERC20Storage storage $ = _getZERC20Storage();
        super._afterTokenTransfer(from, to, value);
        $.hashChain = ShaHashChainLib.compute($.hashChain, to, value);
        emit IndexedTransfer($.index++, from, to, value);
    }

    /// @notice Sets the Verifier contract that is allowed to relay teleport mints.
    /// @dev Prevents the zero address because the Verifier role is mandatory for teleport mints.
    /// @param newVerifier LayerZero-aware Verifier contract.
    function setVerifier(address newVerifier) external onlyOwner {
        if (newVerifier == address(0)) revert ZeroAddress();
        _getZERC20Storage().verifier = newVerifier;
        emit VerifierUpdated(newVerifier);
    }

    /// @notice Sets the Minter contract that can mint/burn to balance deposit liquidity.
    /// @dev Unlike verifier, the spec allows disabling the minter by setting address(0) on chains without deposits.
    /// @param newMinter Contract that exercises `mint`/`burn` for bridge deposits.
    function setMinter(address newMinter) external onlyOwner {
        _getZERC20Storage().minter = newMinter;
        emit MinterUpdated(newMinter);
    }

    /// @notice Mints tokens under the Minter role defined by the deposit / redemption flow.
    /// @param to Recipient of the freshly minted zERC20.
    /// @param value Amount minted 1:1 with deposited liquidity.
    function mint(address to, uint256 value) external {
        if (msg.sender != minter()) revert OnlyMinter();
        _mint(to, value);
    }

    /// @notice Burns tokens under the Minter role prior to native/ERC20 withdrawals.
    /// @param from Holder whose balance is reduced to release the underlying asset.
    /// @param value Amount burned 1:1 with withdrawn liquidity.
    function burn(address from, uint256 value) external {
        if (msg.sender != minter()) revert OnlyMinter();
        _burn(from, value);
    }
}
