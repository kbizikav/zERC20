// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {IPreCrime} from "@layerzerolabs/lz-evm-oapp-v2/contracts/precrime/interfaces/IPreCrime.sol";
import {
    IOAppPreCrimeSimulator,
    InboundPacket,
    Origin
} from "@layerzerolabs/lz-evm-oapp-v2/contracts/precrime/interfaces/IOAppPreCrimeSimulator.sol";

/**
 * @title OAppPreCrimeSimulatorUpgradeable
 * @notice Upgradeable variant of the LayerZero pre-crime simulator helper.
 * @dev Ownership is expected to be initialized by an upstream initializer.
 */
abstract contract OAppPreCrimeSimulatorUpgradeable is Initializable, OwnableUpgradeable, IOAppPreCrimeSimulator {
    address public preCrime;

    /// forge-lint: disable-next-line(mixed-case-function)
    function __OAppPreCrimeSimulator_init() internal onlyInitializing {}

    function oApp() external view virtual returns (address) {
        return address(this);
    }

    function setPreCrime(address _preCrime) public virtual onlyOwner {
        preCrime = _preCrime;
        emit PreCrimeSet(_preCrime);
    }

    function lzReceiveAndRevert(InboundPacket[] calldata _packets) public payable virtual {
        for (uint256 i = 0; i < _packets.length; i++) {
            InboundPacket calldata packet = _packets[i];

            if (!isPeer(packet.origin.srcEid, packet.origin.sender)) continue;

            this.lzReceiveSimulate{value: packet.value}(
                packet.origin, packet.guid, packet.message, packet.executor, packet.extraData
            );
        }

        revert SimulationResult(IPreCrime(msg.sender).buildSimulationResult());
    }

    function lzReceiveSimulate(
        Origin calldata _origin,
        bytes32 _guid,
        bytes calldata _message,
        address _executor,
        bytes calldata _extraData
    ) external payable virtual {
        if (msg.sender != address(this)) revert OnlySelf();
        _lzReceiveSimulate(_origin, _guid, _message, _executor, _extraData);
    }

    function _lzReceiveSimulate(
        Origin calldata _origin,
        bytes32 _guid,
        bytes calldata _message,
        address _executor,
        bytes calldata _extraData
    ) internal virtual;

    function isPeer(uint32 _eid, bytes32 _peer) public view virtual returns (bool);

    uint256[49] private __gap;
}
