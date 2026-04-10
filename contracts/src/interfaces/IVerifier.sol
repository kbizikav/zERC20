// SPDX-License-Identifier: BUSL-1.1
pragma solidity ^0.8.24;

interface IWithdrawVerifier {
    function verifyProof(
        uint256[2] calldata _pA,
        uint256[2][2] calldata _pB,
        uint256[2] calldata _pC,
        uint256[3] calldata _pubSignals
    ) external view returns (bool);
}
