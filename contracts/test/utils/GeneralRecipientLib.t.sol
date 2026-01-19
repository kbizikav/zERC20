// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {Test} from "forge-std/Test.sol";
import {GeneralRecipientLib} from "../../src/utils/GeneralRecipientLib.sol";

contract GeneralRecipientLibHarness {
    function hash(GeneralRecipientLib.GeneralRecipient calldata gr) external pure returns (uint256) {
        return GeneralRecipientLib.hash(gr);
    }

    function version() external pure returns (uint8) {
        return GeneralRecipientLib.VERSION;
    }
}

contract GeneralRecipientLibTest is Test {
    GeneralRecipientLibHarness internal lib;

    function setUp() public {
        lib = new GeneralRecipientLibHarness();
    }

    // ==================== Version Tests ====================

    function testVersionIsOne() public view {
        assertEq(lib.version(), 1, "VERSION should be 1");
    }

    // ==================== Hash Tests ====================

    function testHashReturnsNonZero() public view {
        GeneralRecipientLib.GeneralRecipient memory gr = GeneralRecipientLib.GeneralRecipient({
            chainId: 1, recipient: bytes32(uint256(uint160(address(0xA11CE)))), tweak: bytes32(uint256(123))
        });

        uint256 result = lib.hash(gr);

        assertGt(result, 0, "hash should be non-zero");
    }

    function testHashHasVersionInUpperBits() public view {
        GeneralRecipientLib.GeneralRecipient memory gr = GeneralRecipientLib.GeneralRecipient({
            chainId: 1, recipient: bytes32(uint256(uint160(address(0xA11CE)))), tweak: bytes32(uint256(123))
        });

        uint256 result = lib.hash(gr);
        uint256 versionByte = result >> 248;

        assertEq(versionByte, 1, "upper 8 bits should contain VERSION");
    }

    function testHashIsDeterministic() public view {
        GeneralRecipientLib.GeneralRecipient memory gr = GeneralRecipientLib.GeneralRecipient({
            chainId: 42, recipient: bytes32(uint256(uint160(address(0xB0B)))), tweak: bytes32(uint256(456))
        });

        uint256 result1 = lib.hash(gr);
        uint256 result2 = lib.hash(gr);

        assertEq(result1, result2, "hash should be deterministic");
    }

    function testHashDiffersForDifferentChainId() public view {
        GeneralRecipientLib.GeneralRecipient memory gr1 = GeneralRecipientLib.GeneralRecipient({
            chainId: 1, recipient: bytes32(uint256(uint160(address(0xA11CE)))), tweak: bytes32(uint256(123))
        });

        GeneralRecipientLib.GeneralRecipient memory gr2 = GeneralRecipientLib.GeneralRecipient({
            chainId: 2, recipient: bytes32(uint256(uint160(address(0xA11CE)))), tweak: bytes32(uint256(123))
        });

        uint256 hash1 = lib.hash(gr1);
        uint256 hash2 = lib.hash(gr2);

        assertNotEq(hash1, hash2, "different chainId should produce different hash");
    }

    function testHashDiffersForDifferentRecipient() public view {
        GeneralRecipientLib.GeneralRecipient memory gr1 = GeneralRecipientLib.GeneralRecipient({
            chainId: 1, recipient: bytes32(uint256(uint160(address(0xA11CE)))), tweak: bytes32(uint256(123))
        });

        GeneralRecipientLib.GeneralRecipient memory gr2 = GeneralRecipientLib.GeneralRecipient({
            chainId: 1, recipient: bytes32(uint256(uint160(address(0xB0B)))), tweak: bytes32(uint256(123))
        });

        uint256 hash1 = lib.hash(gr1);
        uint256 hash2 = lib.hash(gr2);

        assertNotEq(hash1, hash2, "different recipient should produce different hash");
    }

    function testHashDiffersForDifferentTweak() public view {
        GeneralRecipientLib.GeneralRecipient memory gr1 = GeneralRecipientLib.GeneralRecipient({
            chainId: 1, recipient: bytes32(uint256(uint160(address(0xA11CE)))), tweak: bytes32(uint256(123))
        });

        GeneralRecipientLib.GeneralRecipient memory gr2 = GeneralRecipientLib.GeneralRecipient({
            chainId: 1, recipient: bytes32(uint256(uint160(address(0xA11CE)))), tweak: bytes32(uint256(456))
        });

        uint256 hash1 = lib.hash(gr1);
        uint256 hash2 = lib.hash(gr2);

        assertNotEq(hash1, hash2, "different tweak should produce different hash");
    }

    function testHashWithZeroValues() public view {
        GeneralRecipientLib.GeneralRecipient memory gr =
            GeneralRecipientLib.GeneralRecipient({chainId: 0, recipient: bytes32(0), tweak: bytes32(0)});

        uint256 result = lib.hash(gr);
        uint256 versionByte = result >> 248;

        assertEq(versionByte, 1, "VERSION should still be in upper bits for zero input");
        assertGt(result, 0, "hash should be non-zero even for zero input");
    }

    function testHashWithMaxValues() public view {
        GeneralRecipientLib.GeneralRecipient memory gr = GeneralRecipientLib.GeneralRecipient({
            chainId: type(uint64).max, recipient: bytes32(type(uint256).max), tweak: bytes32(type(uint256).max)
        });

        uint256 result = lib.hash(gr);
        uint256 versionByte = result >> 248;

        assertEq(versionByte, 1, "VERSION should still be in upper bits for max input");
    }

    function testHashLower248BitsAreMasked() public view {
        GeneralRecipientLib.GeneralRecipient memory gr = GeneralRecipientLib.GeneralRecipient({
            chainId: 1, recipient: bytes32(uint256(uint160(address(0xA11CE)))), tweak: bytes32(uint256(123))
        });

        uint256 result = lib.hash(gr);

        // Verify that the upper 8 bits only contain the version
        uint256 upper8Bits = result >> 248;
        assertEq(upper8Bits, 1, "upper 8 bits should only contain VERSION");

        // Verify that the lower 248 bits are from sha256
        uint256 lower248Bits = result & 0x00FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF;
        assertGt(lower248Bits, 0, "lower 248 bits should be non-zero");
    }

    // ==================== Fuzz Tests ====================

    function testFuzzHashAlwaysHasVersionByte(uint64 chainId, bytes32 recipient, bytes32 tweak) public view {
        GeneralRecipientLib.GeneralRecipient memory gr =
            GeneralRecipientLib.GeneralRecipient({chainId: chainId, recipient: recipient, tweak: tweak});

        uint256 result = lib.hash(gr);
        uint256 versionByte = result >> 248;

        assertEq(versionByte, 1, "VERSION should always be 1");
    }

    function testFuzzHashIsDeterministic(uint64 chainId, bytes32 recipient, bytes32 tweak) public view {
        GeneralRecipientLib.GeneralRecipient memory gr =
            GeneralRecipientLib.GeneralRecipient({chainId: chainId, recipient: recipient, tweak: tweak});

        uint256 result1 = lib.hash(gr);
        uint256 result2 = lib.hash(gr);

        assertEq(result1, result2, "hash should be deterministic");
    }

    function testFuzzDifferentInputsProduceDifferentHashes(
        uint64 chainId1,
        uint64 chainId2,
        bytes32 recipient,
        bytes32 tweak
    ) public view {
        vm.assume(chainId1 != chainId2);

        GeneralRecipientLib.GeneralRecipient memory gr1 =
            GeneralRecipientLib.GeneralRecipient({chainId: chainId1, recipient: recipient, tweak: tweak});

        GeneralRecipientLib.GeneralRecipient memory gr2 =
            GeneralRecipientLib.GeneralRecipient({chainId: chainId2, recipient: recipient, tweak: tweak});

        uint256 hash1 = lib.hash(gr1);
        uint256 hash2 = lib.hash(gr2);

        assertNotEq(hash1, hash2, "different inputs should produce different hashes");
    }
}
