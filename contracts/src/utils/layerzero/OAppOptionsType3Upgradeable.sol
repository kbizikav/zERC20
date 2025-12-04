// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {IOAppOptionsType3, EnforcedOptionParam} from "@layerzerolabs/lz-evm-oapp-v2/contracts/oapp/interfaces/IOAppOptionsType3.sol";

/**
 * @title OAppOptionsType3Upgradeable
 * @notice Upgradeable variant of LayerZero's OAppOptionsType3 helper.
 * @dev Does not invoke `__Ownable_init` because ownership is expected to be initialized upstream.
 */
abstract contract OAppOptionsType3Upgradeable is Initializable, OwnableUpgradeable, IOAppOptionsType3 {
    uint16 internal constant OPTION_TYPE_3 = 3;

    // msgType => enforced options per endpoint
    mapping(uint32 eid => mapping(uint16 msgType => bytes enforcedOption)) public enforcedOptions;

    /// forge-lint: disable-next-line(mixed-case-function)
    function __OAppOptionsType3_init() internal onlyInitializing {}

    function setEnforcedOptions(EnforcedOptionParam[] calldata _enforcedOptions) public virtual onlyOwner {
        _setEnforcedOptions(_enforcedOptions);
    }

    function _setEnforcedOptions(EnforcedOptionParam[] memory _enforcedOptions) internal virtual {
        for (uint256 i = 0; i < _enforcedOptions.length; i++) {
            _assertOptionsType3(_enforcedOptions[i].options);
            enforcedOptions[_enforcedOptions[i].eid][_enforcedOptions[i].msgType] = _enforcedOptions[i].options;
        }

        emit EnforcedOptionSet(_enforcedOptions);
    }

    function combineOptions(uint32 _eid, uint16 _msgType, bytes calldata _extraOptions)
        public
        view
        virtual
        returns (bytes memory)
    {
        bytes memory enforced = enforcedOptions[_eid][_msgType];

        if (enforced.length == 0) return _extraOptions;
        if (_extraOptions.length == 0) return enforced;

        if (_extraOptions.length >= 2) {
            _assertOptionsType3(_extraOptions);
            return bytes.concat(enforced, _extraOptions[2:]);
        }

        revert InvalidOptions(_extraOptions);
    }

    function _assertOptionsType3(bytes memory _options) internal pure virtual {
        uint16 optionsType;
        assembly {
            optionsType := mload(add(_options, 2))
        }
        if (optionsType != OPTION_TYPE_3) revert InvalidOptions(_options);
    }

    uint256[49] private __gap;
}
