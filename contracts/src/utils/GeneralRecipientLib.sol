// SPDX-License-Identifier: BUSL-1.1
pragma solidity ^0.8.24;

/// @title GeneralRecipientLib
/// @notice Library for computing versioned recipient hashes used in ZK proof verification.
library GeneralRecipientLib {
    /// @notice Version byte embedded in the upper 8 bits of the hash.
    uint8 internal constant VERSION = 1;

    /// @dev Mask to clear upper 8 bits, keeping lower 248 bits of the hash.
    uint256 private constant LOWER_248_BITS_MASK = type(uint256).max >> 8;

    /// @notice Represents a cross-chain recipient with chain binding and privacy tweak.
    /// @param chainId Target chain identifier for recipient validation.
    /// @param recipient Address of the recipient encoded as bytes32.
    /// @param tweak Privacy tweak to prevent address correlation across contexts.
    struct GeneralRecipient {
        uint64 chainId;
        bytes32 recipient;
        bytes32 tweak;
    }

    /// @notice Computes a versioned hash of a GeneralRecipient for ZK proof verification.
    /// @dev Uses sha256 with upper 8 bits reserved for version byte.
    ///      The hash format is: [VERSION (8 bits)][sha256 digest (248 bits)]
    /// @param gr The recipient struct containing chainId, recipient address, and tweak.
    /// @return A 256-bit hash with VERSION in the upper 8 bits.
    function hash(GeneralRecipient memory gr) internal pure returns (uint256) {
        bytes32 digest = sha256(abi.encodePacked(gr.chainId, gr.recipient, gr.tweak));
        uint256 masked = uint256(digest) & LOWER_248_BITS_MASK;
        return masked | (uint256(VERSION) << 248);
    }
}
