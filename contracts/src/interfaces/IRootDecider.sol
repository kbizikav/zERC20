// SPDX-License-Identifier: BUSL-1.1
pragma solidity ^0.8.24;

interface IRootDecider {
    function verifyOpaqueNovaProof(uint256[32] calldata proof) external view returns (bool);
}
