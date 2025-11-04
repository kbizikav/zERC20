// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {
    MessagingParams,
    MessagingReceipt,
    MessagingFee,
    Origin
} from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/ILayerZeroEndpointV2.sol";

/// @notice Lightweight testing harness that mimics the LayerZero Endpoint V2 surface expected by the OZv5 OApps.
abstract contract TestHelperOz5 is Test {
    /// @dev Deploys a fresh endpoint mock with the provided endpoint id.
    function _deployEndpoint(uint32 eid) internal returns (EndpointV2Mock endpoint) {
        endpoint = new EndpointV2Mock(eid);
    }

    /// @dev Deploys a simple send library placeholder.
    function _deployMessageLib() internal returns (MockSendLib lib) {
        lib = new MockSendLib();
    }
}

/// @notice Minimal stand-in for the LayerZero EndpointV2 contract that supports the behaviours exercised in tests.
contract EndpointV2Mock {
    struct FeeConfig {
        uint256 nativeFee;
        uint256 lzTokenFee;
    }

    uint32 public immutable EID;
    uint64 private _nonce;

    mapping(address => bool) public delegates;
    mapping(uint32 => address) public defaultSendLibrary;
    mapping(uint32 => address) public defaultReceiveLibrary;
    mapping(uint32 => FeeConfig) private feeByEid;

    event PacketSent(bytes encodedPayload, bytes options, address sendLibrary);
    event DelegateSet(address sender, address delegate);

    constructor(uint32 eid_) {
        EID = eid_;
    }

    function setDelegate(address delegate) external {
        delegates[delegate] = true;
        emit DelegateSet(msg.sender, delegate);
    }

    function registerLibrary(address) external {}

    function setDefaultSendLibrary(uint32 dstEid, address lib) external {
        defaultSendLibrary[dstEid] = lib;
    }

    function setDefaultReceiveLibrary(uint32 dstEid, address lib, uint256) external {
        defaultReceiveLibrary[dstEid] = lib;
    }

    function setMessagingFee(uint32 dstEid, uint256 nativeFee, uint256 lzTokenFee) external {
        feeByEid[dstEid] = FeeConfig({nativeFee: nativeFee, lzTokenFee: lzTokenFee});
    }

    function quote(MessagingParams calldata params, address) external view returns (MessagingFee memory fee) {
        FeeConfig memory cfg = feeByEid[params.dstEid];
        return MessagingFee({nativeFee: cfg.nativeFee, lzTokenFee: cfg.lzTokenFee});
    }

    function send(MessagingParams calldata params, address)
        external
        payable
        returns (MessagingReceipt memory receipt)
    {
        FeeConfig memory cfg = feeByEid[params.dstEid];
        bytes memory packed = abi.encode(params.dstEid, params.receiver, params.message);
        emit PacketSent(packed, params.options, defaultSendLibrary[params.dstEid]);

        uint64 nextNonce = ++_nonce;
        bytes32 guid = keccak256(abi.encode(packed, nextNonce, msg.value));
        MessagingFee memory fee = MessagingFee({nativeFee: cfg.nativeFee, lzTokenFee: cfg.lzTokenFee});
        return MessagingReceipt({guid: guid, nonce: nextNonce, fee: fee});
    }

    function lzReceive(Origin calldata, address, bytes32, bytes calldata, bytes calldata) external payable {}

    function verify(Origin calldata, address, bytes32) external pure {}

    function verifiable(Origin calldata, address) external pure returns (bool) {
        return true;
    }

    function initializable(Origin calldata, address) external pure returns (bool) {
        return true;
    }

    function clear(address, Origin calldata, bytes32, bytes calldata) external pure {}

    function setLzToken(address) external pure {}

    function lzToken() external pure returns (address) {
        return address(0);
    }

    function nativeToken() external pure returns (address) {
        return address(0);
    }
}

/// @notice Placeholder contract used to represent a configured messaging library.
contract MockSendLib {
    function isSupportedEid(uint32) external pure returns (bool) {
        return true;
    }
}
