// SPDX-License-Identifier: Unlicense
pragma solidity ^0.8.20;

library GeneralRecipientLib {
    uint8 internal constant VERSION = 1;

    struct GeneralRecipient {
        uint64 chainId;
        bytes32 recipient;
        bytes32 tweak;
    }

    function hash(GeneralRecipient memory gr) internal pure returns (uint256) {
        bytes32 digest = sha256(abi.encodePacked(gr.chainId, gr.recipient, gr.tweak));
        uint256 masked = uint256(digest) & 0x00FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF;
        return masked | (uint256(VERSION) << 248);
    }
}
