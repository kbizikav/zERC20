// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {
    SafeERC20Upgradeable,
    IERC20Upgradeable
} from "@openzeppelin/contracts-upgradeable/token/ERC20/utils/SafeERC20Upgradeable.sol";
import {
    MessagingParams,
    MessagingFee,
    MessagingReceipt
} from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/ILayerZeroEndpointV2.sol";
import {OAppCoreUpgradeable} from "./OAppCoreUpgradeable.sol";
import {IOAppCore} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oapp/interfaces/IOAppCore.sol";

/**
 * @title OAppSenderUpgradeable
 * @notice Upgradeable variant of LayerZero's OAppSender.
 */
abstract contract OAppSenderUpgradeable is OAppCoreUpgradeable {
    using SafeERC20Upgradeable for IERC20Upgradeable;

    error NotEnoughNative(uint256 msgValue);
    error LzTokenUnavailable();

    uint64 internal constant SENDER_VERSION = 1;

    /// @inheritdoc IOAppCore
    function oAppVersion() public view virtual override returns (uint64 senderVersion, uint64 receiverVersion) {
        return (SENDER_VERSION, 0);
    }

    function _quote(uint32 _dstEid, bytes memory _message, bytes memory _options, bool _payInLzToken)
        internal
        view
        virtual
        returns (MessagingFee memory fee)
    {
        return endpoint.quote(
            MessagingParams(_dstEid, _getPeerOrRevert(_dstEid), _message, _options, _payInLzToken), address(this)
        );
    }

    function _lzSend(
        uint32 _dstEid,
        bytes memory _message,
        bytes memory _options,
        MessagingFee memory _fee,
        address _refundAddress
    ) internal virtual returns (MessagingReceipt memory receipt) {
        uint256 messageValue = _payNative(_fee.nativeFee);
        if (_fee.lzTokenFee > 0) _payLzToken(_fee.lzTokenFee);

        return endpoint.send{value: messageValue}(
            MessagingParams(_dstEid, _getPeerOrRevert(_dstEid), _message, _options, _fee.lzTokenFee > 0), _refundAddress
        );
    }

    function _payNative(uint256 _nativeFee) internal virtual returns (uint256 nativeFee) {
        if (msg.value != _nativeFee) revert NotEnoughNative(msg.value);
        return _nativeFee;
    }

    function _payLzToken(uint256 _lzTokenFee) internal virtual {
        address lzToken = endpoint.lzToken();
        if (lzToken == address(0)) revert LzTokenUnavailable();

        IERC20Upgradeable(lzToken).safeTransferFrom(msg.sender, address(endpoint), _lzTokenFee);
    }

    uint256[48] private __gap;
}
