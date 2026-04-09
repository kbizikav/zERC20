// SPDX-License-Identifier: BUSL-1.1
pragma solidity ^0.8.24;

interface IWithdrawDecider {
    function verifyOpaqueNovaProof(uint256[34] calldata proof) external view returns (bool);
}
