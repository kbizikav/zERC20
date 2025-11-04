// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IMintableBurnableERC20} from "./interfaces/IMintableBurnableERC20.sol";
import {IERC20Upgradeable} from "@openzeppelin/contracts-upgradeable/token/ERC20/IERC20Upgradeable.sol";
import {SafeERC20Upgradeable} from "@openzeppelin/contracts-upgradeable/token/ERC20/utils/SafeERC20Upgradeable.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {ReentrancyGuardUpgradeable} from "@openzeppelin/contracts-upgradeable/security/ReentrancyGuardUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";

/**
 * @title Minter
 * @notice Handles wrapping and unwrapping of native or ERC20 tokens through a mintable zerc20 token.
 * @dev The contract is UUPS-upgradeable and supports either native token deposits or ERC20 deposits
 *      based on the configured `tokenAddress` (zero for native, non-zero for ERC20).
 */
contract Minter is OwnableUpgradeable, UUPSUpgradeable, ReentrancyGuardUpgradeable {
    using SafeERC20Upgradeable for IERC20Upgradeable;

    /// @notice Emitted when native assets are deposited and wrapped.
    event NativeDeposited(address indexed account, uint256 amount);
    /// @notice Emitted when ERC20 assets are deposited and wrapped.
    event TokenDeposited(address indexed account, uint256 amount);
    /// @notice Emitted when native assets are unwrapped and withdrawn.
    event NativeWithdrawn(address indexed account, uint256 amount);
    /// @notice Emitted when ERC20 assets are unwrapped and withdrawn.
    event TokenWithdrawn(address indexed account, uint256 amount);

    /// ---------------------------------------------------------------------
    /// Errors
    /// ---------------------------------------------------------------------

    error ZeroZerc20Token();
    error ZeroOwner();
    error NativeDisabled();
    error TokenDisabled();
    error ZeroAmount();
    error NativeTransferFailed();
    error InsufficientNativeLiquidity(uint256 available, uint256 requested);
    error InsufficientTokenLiquidity(uint256 available, uint256 requested);

    /// ---------------------------------------------------------------------
    /// Storage
    /// ---------------------------------------------------------------------

    /// @notice Address of the zerc20 token that exposes mint / burn functions.
    address public zerc20Token;
    /// @notice Address of the underlying token being wrapped (zero address represents native token).
    address public tokenAddress;

    constructor() {
        _disableInitializers();
    }

    /// @notice Initializes the contract with the wrapper and underlying token addresses.
    /// @param zerc20Token_ Address of the mintable/burnable zerc20 token (must be non-zero).
    /// @param tokenAddress_ Address of the underlying ERC20 token (zero when wrapping native token).
    /// @param initialOwner Address that will assume ownership for upgrades and administration.
    function initialize(address zerc20Token_, address tokenAddress_, address initialOwner)
        external
        initializer
    {
        if (zerc20Token_ == address(0)) revert ZeroZerc20Token();
        if (initialOwner == address(0)) revert ZeroOwner();

        __Ownable_init();
        __UUPSUpgradeable_init();
        __ReentrancyGuard_init();

        zerc20Token = zerc20Token_;
        tokenAddress = tokenAddress_;

        _transferOwnership(initialOwner);
    }

    /// ---------------------------------------------------------------------
    /// Deposits
    /// ---------------------------------------------------------------------

    /// @notice Deposit native currency and receive wrapped tokens.
    /// @dev Only callable when `tokenAddress` is zero.
    function depositNative() external payable nonReentrant {
        if (tokenAddress != address(0)) revert NativeDisabled();
        uint256 amount = msg.value;
        if (amount == 0) revert ZeroAmount();

        IMintableBurnableERC20(zerc20Token).mint(msg.sender, amount);
        emit NativeDeposited(msg.sender, amount);
    }

    /// @notice Deposit ERC20 tokens and receive wrapped tokens.
    /// @dev Only callable when `tokenAddress` is non-zero.
    /// @param amount Quantity of underlying tokens to deposit.
    function depositToken(uint256 amount) external nonReentrant {
        if (tokenAddress == address(0)) revert TokenDisabled();
        if (amount == 0) revert ZeroAmount();

        IERC20Upgradeable(tokenAddress).safeTransferFrom(msg.sender, address(this), amount);
        IMintableBurnableERC20(zerc20Token).mint(msg.sender, amount);

        emit TokenDeposited(msg.sender, amount);
    }

    /// ---------------------------------------------------------------------
    /// Withdrawals
    /// ---------------------------------------------------------------------

    /// @notice Burn wrapped tokens and withdraw native currency.
    /// @dev Only callable when `tokenAddress` is zero.
    /// @param amount Quantity of wrapped tokens to burn / native currency to redeem.
    function withdrawNative(uint256 amount) external nonReentrant {
        if (tokenAddress != address(0)) revert NativeDisabled();
        if (amount == 0) revert ZeroAmount();

        uint256 available = address(this).balance;
        if (available < amount) revert InsufficientNativeLiquidity(available, amount);

        IMintableBurnableERC20(zerc20Token).burn(msg.sender, amount);

        (bool success,) = msg.sender.call{value: amount}("");
        if (!success) revert NativeTransferFailed();

        emit NativeWithdrawn(msg.sender, amount);
    }

    /// @notice Burn wrapped tokens and withdraw the underlying ERC20.
    /// @dev Only callable when `tokenAddress` is non-zero.
    /// @param amount Quantity of wrapped tokens to burn / ERC20 to redeem.
    function withdrawToken(uint256 amount) external nonReentrant {
        if (tokenAddress == address(0)) revert TokenDisabled();
        if (amount == 0) revert ZeroAmount();

        uint256 available = IERC20Upgradeable(tokenAddress).balanceOf(address(this));
        if (available < amount) revert InsufficientTokenLiquidity(available, amount);

        IMintableBurnableERC20(zerc20Token).burn(msg.sender, amount);
        IERC20Upgradeable(tokenAddress).safeTransfer(msg.sender, amount);

        emit TokenWithdrawn(msg.sender, amount);
    }

    /// ---------------------------------------------------------------------
    /// Upgrades
    /// ---------------------------------------------------------------------

    function _authorizeUpgrade(address) internal override onlyOwner {}

    uint256[46] private __gap;
}
