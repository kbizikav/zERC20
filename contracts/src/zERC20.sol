// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IzERC20} from "./interfaces/IzERC20.sol";
import {ERC20Upgradeable} from "@openzeppelin/contracts-upgradeable/token/ERC20/ERC20Upgradeable.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {ShaHashChainLib} from "./utils/ShaHashChainLib.sol";

/// @title zERC20
/// @notice Upgradeable ERC20 token that supports private proof of burn.
contract zERC20 is ERC20Upgradeable, OwnableUpgradeable, UUPSUpgradeable, IzERC20 {
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

    /// @notice Hash chain committing every transfer's destination and value pair.
    uint256 public hashChain;

    /// @notice Index of the next transfer, matching the off-chain Merkle tree leaf position.
    uint256 public index;

    /// @notice Address allowed to call verifier-only functions such as teleport.
    address public verifier;

    /// @notice Address allowed to mint and burn under the minter role.
    address public minter;

    /// @notice Locks implementation contracts on deployment.
    constructor() {
        _disableInitializers();
    }

    /// @notice Initializes token metadata and ownership.
    /// @param name_ ERC20 name.
    /// @param symbol_ ERC20 symbol.
    /// @param initialOwner Account receiving ownership and upgrade authority.
    function initialize(string memory name_, string memory symbol_, address initialOwner) external initializer {
        if (initialOwner == address(0)) revert ZeroAddress();

        __ERC20_init(name_, symbol_);
        __Ownable_init();
        __UUPSUpgradeable_init();

        _transferOwnership(initialOwner);
    }

    /// @dev Restricts upgrade authorization to the owner.
    function _authorizeUpgrade(address) internal override onlyOwner {}

    /// @inheritdoc IzERC20
    /// @dev Only the verifier can call this teleport entrypoint.
    function teleport(address to, uint256 value) external {
        if (msg.sender != verifier) revert OnlyVerifier();
        _mint(to, value);
        emit Teleport(to, value);
    }

    /// @dev After-transfer hook that commits the recipient and amount to the hash chain.
    function _afterTokenTransfer(address from, address to, uint256 value) internal override {
        require(value <= type(uint248).max, ValueTooLarge());
        super._afterTokenTransfer(from, to, value);
        hashChain = ShaHashChainLib.compute(hashChain, to, value);
        emit IndexedTransfer(index++, from, to, value);
    }

    /// @notice Updates the verifier address; callable only by the owner.
    function setVerifier(address newVerifier) external onlyOwner {
        if (newVerifier == address(0)) revert ZeroAddress();
        verifier = newVerifier;
        emit VerifierUpdated(newVerifier);
    }

    /// @notice Updates the minter address; callable only by the owner.
    function setMinter(address newMinter) external onlyOwner {
        if (newMinter == address(0)) revert ZeroAddress();
        minter = newMinter;
        emit MinterUpdated(newMinter);
    }

    /// @notice Mints tokens under the minter role.
    function mint(address to, uint256 value) external {
        if (msg.sender != minter) revert OnlyMinter();
        _mint(to, value);
    }

    /// @notice Burns tokens under the minter role.
    function burn(address from, uint256 value) external {
        if (msg.sender != minter) revert OnlyMinter();
        _burn(from, value);
    }

    /// @dev Storage gap reserved for future upgrades.
    uint256[45] private __gap;
}
