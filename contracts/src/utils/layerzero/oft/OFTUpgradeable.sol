// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {ERC20Upgradeable} from "@openzeppelin/contracts-upgradeable/token/ERC20/ERC20Upgradeable.sol";
import {OFTCoreUpgradeable} from "./OFTCoreUpgradeable.sol";

/**
 * @title OFTUpgradeable
 * @notice Upgradeable ERC20 implementation of the LayerZero OFT standard.
 */
abstract contract OFTUpgradeable is Initializable, OFTCoreUpgradeable, ERC20Upgradeable {
    /// forge-lint: disable-next-line(mixed-case-function)
    function __OFT_init(string memory _name, string memory _symbol, address _endpoint, address _delegate)
        internal
        onlyInitializing
    {
        __ERC20_init(_name, _symbol);
        __OFTCore_init(decimals(), _endpoint, _delegate);
    }

    /// forge-lint: disable-next-line(mixed-case-function)
    function __OFT_init_unchained(string memory, string memory, address, address) internal onlyInitializing {}

    function token() public view returns (address) {
        return address(this);
    }

    function approvalRequired() external pure virtual returns (bool) {
        return false;
    }

    function _debit(address _from, uint256 _amountLD, uint256 _minAmountLD, uint32 _dstEid)
        internal
        virtual
        override
        returns (uint256 amountSentLD, uint256 amountReceivedLD)
    {
        (amountSentLD, amountReceivedLD) = _debitView(_amountLD, _minAmountLD, _dstEid);
        _burn(_from, amountSentLD);
    }

    function _credit(address _to, uint256 _amountLD, uint32 /*_srcEid*/ )
        internal
        virtual
        override
        returns (uint256 amountReceivedLD)
    {
        if (_to == address(0x0)) _to = address(0xdead);
        _mint(_to, _amountLD);
        return _amountLD;
    }

    uint256[50] private __gap;
}
