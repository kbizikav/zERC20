// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC20Permit} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Permit.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";

/// @title SwapHelper
/// @notice Atomically executes permit + transferFrom + native transfer in a single transaction.
///         Works with any ERC20Permit-compatible token. Deploy one per chain.
///         Only allowlisted relayers may call `swap()`.
contract SwapHelper is OwnableUpgradeable, UUPSUpgradeable {
    error NativeTransferFailed();
    error NotAllowlisted();

    event RelayerUpdated(address indexed relayer, bool allowed);

    /// @custom:storage-location erc7201:zerc20.storage.SwapHelper
    struct SwapHelperStorage {
        mapping(address => bool) allowlisted;
    }

    // keccak256(abi.encode(uint256(keccak256("zerc20.storage.SwapHelper")) - 1)) & ~bytes32(uint256(0xff))
    bytes32 private constant SWAP_HELPER_STORAGE_SLOT =
        0xe2a8e7f0498d60758f328fe366e0da4aeab70b4ec0e634011723dd6a9172c100;

    function _getSwapHelperStorage() private pure returns (SwapHelperStorage storage $) {
        assembly {
            $.slot := SWAP_HELPER_STORAGE_SLOT
        }
    }

    /// @custom:oz-upgrades-unsafe-allow constructor
    constructor() {
        _disableInitializers();
    }

    function initialize(address owner_) external initializer {
        __Ownable_init(owner_);
    }

    // -----------------------------------------------------------------------
    // Allowlist management (owner only)
    // -----------------------------------------------------------------------

    function setRelayer(address relayer, bool allowed) external onlyOwner {
        _getSwapHelperStorage().allowlisted[relayer] = allowed;
        emit RelayerUpdated(relayer, allowed);
    }

    function isRelayer(address relayer) external view returns (bool) {
        return _getSwapHelperStorage().allowlisted[relayer];
    }

    // -----------------------------------------------------------------------
    // Swap
    // -----------------------------------------------------------------------

    /// @notice Execute a token-to-native swap.
    /// @dev The caller (relayer) sends msg.value which is forwarded entirely to `recipient`.
    ///      Tokens are transferred from `owner` to `msg.sender` (relayer).
    ///      Only allowlisted relayers may call this function.
    function swap(
        address token,
        address owner,
        address recipient,
        uint256 tokenAmount,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external payable {
        if (!_getSwapHelperStorage().allowlisted[msg.sender]) revert NotAllowlisted();

        // 1. permit (try/catch: handles front-running and existing allowance)
        try IERC20Permit(token).permit(owner, address(this), tokenAmount, deadline, v, r, s) {} catch {}

        // 2. transferFrom: owner -> msg.sender (relayer receives tokens)
        IERC20(token).transferFrom(owner, msg.sender, tokenAmount);

        // 3. forward entire msg.value to recipient
        (bool ok,) = recipient.call{value: msg.value}("");
        if (!ok) revert NativeTransferFailed();
    }

    // -----------------------------------------------------------------------
    // UUPS
    // -----------------------------------------------------------------------

    function _authorizeUpgrade(address) internal override onlyOwner {}
}
