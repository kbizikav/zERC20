// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IRootDecider {
    function verifyOpaqueNovaProof(uint256[32] calldata proof) external view returns (bool);
}
