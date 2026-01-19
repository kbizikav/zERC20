// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IWithdrawDecider {
    function verifyOpaqueNovaProof(uint256[34] calldata proof) external view returns (bool);
}
