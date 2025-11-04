// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {IOAppCore, ILayerZeroEndpointV2} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oapp/interfaces/IOAppCore.sol";

/**
 * @title OAppCoreUpgradeable
 * @notice Upgradeable variant of LayerZero's OAppCore with initializer-based setup.
 */
abstract contract OAppCoreUpgradeable is Initializable, OwnableUpgradeable, IOAppCore {
    ILayerZeroEndpointV2 public endpoint;
    mapping(uint32 eid => bytes32 peer) public peers;

    /**
     * @notice Initializes the core OApp state.
     * @dev Must be called during contract initialization.
     */
    /// forge-lint: disable-next-line(mixed-case-function)
    function __OAppCore_init(address _endpoint, address _delegate) internal onlyInitializing {
        if (_endpoint == address(0)) revert InvalidEndpointCall();
        if (_delegate == address(0)) revert InvalidDelegate();

        __Ownable_init();
        endpoint = ILayerZeroEndpointV2(_endpoint);

        _transferOwnership(_delegate);
        endpoint.setDelegate(_delegate);
    }

    /**
     * @inheritdoc IOAppCore
     */
    function setPeer(uint32 _eid, bytes32 _peer) public virtual override onlyOwner {
        _setPeer(_eid, _peer);
    }

    /**
     * @notice Internal helper for peer assignment.
     */
    function _setPeer(uint32 _eid, bytes32 _peer) internal virtual {
        peers[_eid] = _peer;
        emit PeerSet(_eid, _peer);
    }

    /**
     * @inheritdoc IOAppCore
     */
    function setDelegate(address _delegate) public virtual override onlyOwner {
        if (_delegate == address(0)) revert InvalidDelegate();
        endpoint.setDelegate(_delegate);
    }

    /**
     * @notice Returns peer for an endpoint or reverts if unset.
     */
    function _getPeerOrRevert(uint32 _eid) internal view virtual returns (bytes32) {
        bytes32 peer = peers[_eid];
        if (peer == bytes32(0)) revert NoPeer(_eid);
        return peer;
    }

    /**
     * @dev Storage gap for future upgrades.
     */
    uint256[50] private __gap;
}
