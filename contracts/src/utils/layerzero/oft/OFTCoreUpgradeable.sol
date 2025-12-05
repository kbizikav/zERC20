// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {OAppUpgradeable} from "../OAppUpgradeable.sol";
import {OAppPreCrimeSimulatorUpgradeable} from "../OAppPreCrimeSimulatorUpgradeable.sol";
import {OAppOptionsType3Upgradeable} from "../OAppOptionsType3Upgradeable.sol";
import {IOAppMsgInspector} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oapp/interfaces/IOAppMsgInspector.sol";
import {IOFT, SendParam, OFTLimit, OFTReceipt, OFTFeeDetail, MessagingReceipt, MessagingFee} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oft/interfaces/IOFT.sol";
import {OFTMsgCodec} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oft/libs/OFTMsgCodec.sol";
import {OFTComposeMsgCodec} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oft/libs/OFTComposeMsgCodec.sol";
import {Origin} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oapp/OApp.sol";

/**
 * @title OFTCoreUpgradeable
 * @notice Upgradeable port of LayerZero's OFTCore.
 */
abstract contract OFTCoreUpgradeable is
    Initializable,
    IOFT,
    OAppUpgradeable,
    OAppPreCrimeSimulatorUpgradeable,
    OAppOptionsType3Upgradeable
{
    using OFTMsgCodec for bytes;
    using OFTMsgCodec for bytes32;

    uint16 public constant SEND = 1;
    uint16 public constant SEND_AND_CALL = 2;

    uint256 public decimalConversionRate;
    address public msgInspector;

    event MsgInspectorSet(address inspector);

    /// forge-lint: disable-next-line(mixed-case-function)
    function __OFTCore_init(uint8 _localDecimals, address _endpoint, address _delegate) internal onlyInitializing {
        if (_localDecimals < sharedDecimals()) revert InvalidLocalDecimals();
        __OApp_init(_endpoint, _delegate);
        __OAppPreCrimeSimulator_init();
        __OAppOptionsType3_init();

        decimalConversionRate = 10 ** (_localDecimals - sharedDecimals());
    }

    function oftVersion() external pure virtual returns (bytes4 interfaceId, uint64 version) {
        return (type(IOFT).interfaceId, 1);
    }

    function sharedDecimals() public view virtual returns (uint8) {
        return 6;
    }

    function setMsgInspector(address _msgInspector) public virtual onlyOwner {
        msgInspector = _msgInspector;
        emit MsgInspectorSet(_msgInspector);
    }

    function quoteOFT(SendParam calldata _sendParam)
        external
        view
        virtual
        returns (OFTLimit memory oftLimit, OFTFeeDetail[] memory oftFeeDetails, OFTReceipt memory oftReceipt)
    {
        uint256 minAmountLD = 0;
        uint256 maxAmountLD = type(uint64).max;
        oftLimit = OFTLimit(minAmountLD, maxAmountLD);

        oftFeeDetails = new OFTFeeDetail[](0);

        (uint256 amountSentLD, uint256 amountReceivedLD) = _debitView(
            _sendParam.amountLD, _sendParam.minAmountLD, _sendParam.dstEid
        );
        oftReceipt = OFTReceipt(amountSentLD, amountReceivedLD);
    }

    function quoteSend(SendParam calldata _sendParam, bool _payInLzToken)
        external
        view
        virtual
        returns (MessagingFee memory msgFee)
    {
        (, uint256 amountReceivedLD) = _debitView(_sendParam.amountLD, _sendParam.minAmountLD, _sendParam.dstEid);
        (bytes memory message, bytes memory options) = _buildMsgAndOptions(_sendParam, amountReceivedLD);
        return _quote(_sendParam.dstEid, message, options, _payInLzToken);
    }

    function send(SendParam calldata _sendParam, MessagingFee calldata _fee, address _refundAddress)
        external
        payable
        virtual
        returns (MessagingReceipt memory msgReceipt, OFTReceipt memory oftReceipt)
    {
        (uint256 amountSentLD, uint256 amountReceivedLD) = _debit(
            msg.sender, _sendParam.amountLD, _sendParam.minAmountLD, _sendParam.dstEid
        );

        (bytes memory message, bytes memory options) = _buildMsgAndOptions(_sendParam, amountReceivedLD);

        msgReceipt = _lzSend(_sendParam.dstEid, message, options, _fee, _refundAddress);
        oftReceipt = OFTReceipt(amountSentLD, amountReceivedLD);

        emit OFTSent(msgReceipt.guid, _sendParam.dstEid, msg.sender, amountSentLD, amountReceivedLD);
    }

    function _buildMsgAndOptions(SendParam calldata _sendParam, uint256 _amountLD)
        internal
        view
        virtual
        returns (bytes memory message, bytes memory options)
    {
        bool hasCompose;
        (message, hasCompose) = OFTMsgCodec.encode(_sendParam.to, _toSD(_amountLD), _sendParam.composeMsg);
        uint16 msgType = hasCompose ? SEND_AND_CALL : SEND;
        options = combineOptions(_sendParam.dstEid, msgType, _sendParam.extraOptions);

        if (msgInspector != address(0)) IOAppMsgInspector(msgInspector).inspect(message, options);
    }

    function _lzReceive(
        Origin calldata _origin,
        bytes32 _guid,
        bytes calldata _message,
        address, /*_executor*/
        bytes calldata /*_extraData*/
    ) internal virtual override {
        address toAddress = _message.sendTo().bytes32ToAddress();
        uint256 amountReceivedLD = _credit(toAddress, _toLD(_message.amountSD()), _origin.srcEid);

        if (_message.isComposed()) {
            bytes memory composeMsg =
                OFTComposeMsgCodec.encode(_origin.nonce, _origin.srcEid, amountReceivedLD, _message.composeMsg());
            endpoint.sendCompose(toAddress, _guid, 0, composeMsg);
        }

        emit OFTReceived(_guid, _origin.srcEid, toAddress, amountReceivedLD);
    }

    function _lzReceiveSimulate(
        Origin calldata _origin,
        bytes32 _guid,
        bytes calldata _message,
        address _executor,
        bytes calldata _extraData
    ) internal virtual override {
        _lzReceive(_origin, _guid, _message, _executor, _extraData);
    }

    function isPeer(uint32 _eid, bytes32 _peer) public view virtual override returns (bool) {
        return peers[_eid] == _peer;
    }

    function _removeDust(uint256 _amountLD) internal view virtual returns (uint256 amountLD) {
        return _amountLD - (_amountLD % decimalConversionRate);
    }

    function _toLD(uint64 _amountSD) internal view virtual returns (uint256 amountLD) {
        return _amountSD * decimalConversionRate;
    }

    function _toSD(uint256 _amountLD) internal view virtual returns (uint64 amountSD) {
        return uint64(_amountLD / decimalConversionRate);
    }

    function _debitView(uint256 _amountLD, uint256 _minAmountLD, uint32 /*_dstEid*/ )
        internal
        view
        virtual
        returns (uint256 amountSentLD, uint256 amountReceivedLD)
    {
        amountSentLD = _removeDust(_amountLD);
        amountReceivedLD = amountSentLD;

        if (amountReceivedLD < _minAmountLD) {
            revert SlippageExceeded(amountReceivedLD, _minAmountLD);
        }
    }

    function _debit(address _from, uint256 _amountLD, uint256 _minAmountLD, uint32 _dstEid)
        internal
        virtual
        returns (uint256 amountSentLD, uint256 amountReceivedLD);

    function _credit(address _to, uint256 _amountLD, uint32 _srcEid)
        internal
        virtual
        returns (uint256 amountReceivedLD);

    uint256[44] private __gap;
}
