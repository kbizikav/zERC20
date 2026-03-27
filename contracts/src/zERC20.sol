// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {IzERC20} from "./interfaces/IzERC20.sol";
import {IBlocklist} from "./interfaces/IBlocklist.sol";
import {ShaHashChainLib} from "./utils/ShaHashChainLib.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {ERC20Upgradeable} from "@openzeppelin/contracts-upgradeable/token/ERC20/ERC20Upgradeable.sol";
import {
    ERC20PermitUpgradeable
} from "@openzeppelin/contracts-upgradeable/token/ERC20/extensions/ERC20PermitUpgradeable.sol";
import {IERC20Permit} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Permit.sol";
import {OFTCoreUpgradeable} from "@layerzerolabs/oft-evm-upgradeable/contracts/oft/OFTCoreUpgradeable.sol";

/// @title zERC20
/// @notice Upgradeable ERC20 token that feeds the zk circuits by enforcing 248-bit transfer values,
///         hashing `(from, to, value)` triples into a SHA-256 chain, and gating mint/burn roles for the Verifier and Minter flows.
///         Also implements the LayerZero V2 OFT interface for omnichain transfers.
// solhint-disable-next-line contract-name-capwords
contract zERC20 is OFTCoreUpgradeable, ERC20PermitUpgradeable, UUPSUpgradeable, IzERC20 {
    uint8 private immutable TOKEN_DECIMALS;
    IBlocklist public immutable BLOCKLIST;

    // ERC-7201 slot for namespace "zerc20.storage.zerc20".
    bytes32 internal constant ZERC20_STORAGE_SLOT = 0xcd5e781c912e334c5bd043d02db19923b6e202919d5c40ac0cfab0473b1e3400;

    /// @custom:storage-location erc7201:zerc20.storage.zerc20
    struct Zerc20Storage {
        uint256 hashChain;
        uint256 index;
        uint256 totalTeleported;
        address verifier;
        address minter;
    }

    function _getZerc20Storage() private pure returns (Zerc20Storage storage $) {
        bytes32 slot = ZERC20_STORAGE_SLOT;
        // solhint-disable-next-line no-inline-assembly
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
    /// @notice Reverts when upgrading to an implementation with a different LayerZero endpoint.
    error EndpointMismatch(address expected, address actual);
    /// @notice Reverts when a blocked address is involved in a transfer.
    error AddressIsBlocked(address account);

    /// @notice Locks implementation contracts on deployment.
    /// @param endpoint LayerZero V2 endpoint address.
    /// @param decimals_ ERC20 decimal precision.
    /// @param blocklist_ Shared OFAC blocklist contract. Must not be address(0).
    constructor(address endpoint, uint8 decimals_, IBlocklist blocklist_) OFTCoreUpgradeable(decimals_, endpoint) {
        require(endpoint != address(0), InvalidEndpointCall());
        require(address(blocklist_) != address(0), ZeroAddress());
        TOKEN_DECIMALS = decimals_;
        BLOCKLIST = blocklist_;
        _disableInitializers();
    }

    /// @notice Initializes token metadata and ownership.
    /// @param name_ ERC20 name.
    /// @param symbol_ ERC20 symbol.
    /// @param initialOwner Account receiving ownership, LayerZero delegate permissions, and upgrade authority.
    function initialize(string calldata name_, string calldata symbol_, address initialOwner) external initializer {
        require(initialOwner != address(0), ZeroAddress());
        __ERC20_init(name_, symbol_);
        __ERC20Permit_init(name_);
        __Ownable_init(initialOwner);
        __OFTCore_init(initialOwner);
    }

    /// @dev Restricts upgrade authorization to the owner and validates endpoint consistency.
    function _authorizeUpgrade(address newImplementation) internal view override onlyOwner {
        address expected = address(endpoint);
        address actual = address(zERC20(newImplementation).endpoint());
        require(actual == expected, EndpointMismatch(expected, actual));
    }

    /// @notice Hash chain committing every transfer's destination and value pair.
    function hashChain() public view returns (uint256) {
        return _getZerc20Storage().hashChain;
    }

    /// @notice Index of the next transfer, matching the off-chain Merkle tree leaf position.
    function index() public view returns (uint256) {
        return _getZerc20Storage().index;
    }

    /// @notice Address allowed to call verifier-only functions such as teleport.
    function verifier() public view returns (address) {
        return _getZerc20Storage().verifier;
    }

    /// @notice Address allowed to mint and burn under the minter role.
    function minter() public view returns (address) {
        return _getZerc20Storage().minter;
    }

    /// @notice Sum of all values minted through verifier-authorized teleports.
    function totalTeleported() public view returns (uint256) {
        return _getZerc20Storage().totalTeleported;
    }

    /// @notice Returns the token decimals.
    function decimals() public view override returns (uint8) {
        return TOKEN_DECIMALS;
    }

    function nonces(address owner) public view override(IERC20Permit, ERC20PermitUpgradeable) returns (uint256) {
        return super.nonces(owner);
    }

    // -----------------------------------------------------------------------
    // OFT Overrides
    // -----------------------------------------------------------------------

    function token() public view override returns (address) {
        return address(this);
    }

    function approvalRequired() external pure override returns (bool) {
        return false;
    }

    function _debit(address from, uint256 amountLd, uint256 minAmountLd, uint32 dstEid)
        internal
        override
        returns (uint256 amountSentLd, uint256 amountReceivedLd)
    {
        (amountSentLd, amountReceivedLd) = _debitView(amountLd, minAmountLd, dstEid);
        _burn(from, amountSentLd);
    }

    function _credit(
        address to,
        uint256 amountLd,
        uint32 /*_srcEid*/
    )
        internal
        override
        returns (uint256 amountReceivedLd)
    {
        // Follow LayerZero OFT convention: redirect address(0) to 0xdead
        // to avoid ERC20 revert while effectively burning the tokens.
        // See: https://github.com/LayerZero-Labs/devtools/.../OFTUpgradeable.sol
        if (to == address(0)) {
            to = address(0xdead);
        }
        _mint(to, amountLd);
        return amountLd;
    }

    // -----------------------------------------------------------------------
    // Teleport / HashChain
    // -----------------------------------------------------------------------

    /// @inheritdoc IzERC20
    /// @dev Called exclusively by the Verifier once a teleport proof succeeds.
    /// @param to Recipient mandated by the zero-knowledge proof (already hashed into the public inputs).
    /// @param value Mint amount corresponding to the delta proven in Verifier.teleport.
    function teleport(address to, uint256 value) external {
        require(msg.sender == verifier(), OnlyVerifier());
        Zerc20Storage storage $ = _getZerc20Storage();
        _mint(to, value);
        $.totalTeleported += value;
        emit Teleport(to, value);
    }

    /// @dev Commits every balance-changing operation to the 248-bit SHA-256 hash chain described in the spec.
    ///      Off-chain/ZKP consumers MUST treat `IndexedTransfer` as the canonical leaf stream (not ERC20 `Transfer`).
    ///      The leaf stream advances on every `_update` invocation (mint/burn/transfer/OFT credit+debit/teleport).
    ///      Note: OFT `_credit` normalizes `to == address(0)` to `address(0xdead)`, and the normalized address is used here.
    ///      Reverts if the amount exceeds the BN254-friendly bound so that the proof circuits remain well-defined.
    function _update(address from, address to, uint256 value) internal override(ERC20Upgradeable) {
        require(value <= type(uint248).max, ValueTooLarge());
        require(!BLOCKLIST.isBlocked(from), AddressIsBlocked(from));
        require(!BLOCKLIST.isBlocked(to), AddressIsBlocked(to));
        super._update(from, to, value);
        Zerc20Storage storage $ = _getZerc20Storage();
        $.hashChain = ShaHashChainLib.compute($.hashChain, from, to, value);
        emit IndexedTransfer($.index, from, to, value);
        ++$.index;
    }

    // -----------------------------------------------------------------------
    // Admin
    // -----------------------------------------------------------------------

    /// @notice Sets the Verifier contract that is allowed to relay teleport mints.
    /// @dev Prevents the zero address because the Verifier role is mandatory for teleport mints.
    /// @param newVerifier LayerZero-aware Verifier contract.
    function setVerifier(address newVerifier) external onlyOwner {
        require(newVerifier != address(0), ZeroAddress());
        _getZerc20Storage().verifier = newVerifier;
        emit VerifierUpdated(newVerifier);
    }

    /// @notice Sets the Minter contract that can mint/burn to balance deposit liquidity.
    /// @dev Unlike verifier, the spec allows disabling the minter by setting address(0) on chains without deposits.
    /// @param newMinter Contract that exercises `mint`/`burn` for bridge deposits.
    function setMinter(address newMinter) external onlyOwner {
        _getZerc20Storage().minter = newMinter;
        emit MinterUpdated(newMinter);
    }

    // -----------------------------------------------------------------------
    // Minter
    // -----------------------------------------------------------------------

    /// @notice Mints tokens under the Minter role defined by the deposit flow.
    /// @dev Reverts if minter is not set (address(0)). This allows chains without
    ///      deposit functionality to disable minting by leaving minter unset.
    /// @param to Recipient of the freshly minted zERC20.
    /// @param value Amount minted 1:1 with deposited liquidity.
    function mint(address to, uint256 value) external {
        require(msg.sender == minter(), OnlyMinter());
        _mint(to, value);
    }

    /// @notice Burns tokens under the Minter role prior to native/ERC20 withdrawals.
    /// @dev Reverts if minter is not set (address(0)). This allows chains without
    ///      withdrawal functionality to disable burning by leaving minter unset.
    /// @param from Holder whose balance is reduced to release the underlying asset.
    /// @param value Amount burned 1:1 with withdrawn liquidity.
    function burn(address from, uint256 value) external {
        require(msg.sender == minter(), OnlyMinter());
        _burn(from, value);
    }
}
