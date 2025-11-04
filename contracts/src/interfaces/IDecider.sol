// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IRootDecider {
    function verifyOpaqueNovaProof(uint256[32] calldata proof) external view returns (bool);
}

interface IWithdrawDecider {
    function verifyOpaqueNovaProof(uint256[34] calldata proof) external view returns (bool);
}
