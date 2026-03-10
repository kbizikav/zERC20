// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";
import {TestHelperOz5, EndpointV2} from "@layerzerolabs/test-devtools-evm-foundry/contracts/TestHelperOz5.sol";
import {Origin} from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/ILayerZeroReceiver.sol";
import {Verifier} from "../../src/Verifier.sol";
import {LiquidityManager} from "../../src/liquidity/LiquidityManager.sol";
import {zERC20} from "../../src/zERC20.sol";
import {IWithdrawDecider} from "../../src/interfaces/IWithdrawDecider.sol";
import {IWithdrawVerifier} from "../../src/interfaces/IVerifier.sol";
import {GeneralRecipientLib} from "../../src/utils/GeneralRecipientLib.sol";
import {IncentiveLib} from "../../src/libraries/IncentiveLib.sol";
import {GelatoRelay} from "../../src/relay/GelatoRelay.sol";
import {GELATO_RELAY_V2} from "relay-context-contracts/constants/GelatoRelay.sol";

// =============================================================================
// Mock contracts
// =============================================================================

contract MockWithdrawDecider is IWithdrawDecider {
    // solhint-disable-next-line gas-calldata-parameters
    function verifyOpaqueNovaProof(uint256[34] memory) external pure returns (bool) {
        return true;
    }
}

contract MockSingleWithdrawVerifier is IWithdrawVerifier {
    function verifyProof(uint256[2] calldata, uint256[2][2] calldata, uint256[2] calldata, uint256[3] calldata)
        external
        pure
        returns (bool)
    {
        return true;
    }
}

contract MintableERC20 is ERC20 {
    constructor() ERC20("Mock USDC", "USDC") {}

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function decimals() public pure override returns (uint8) {
        return 18;
    }
}

// =============================================================================
// Test contract
// =============================================================================

contract GelatoRelayTest is TestHelperOz5 {
    GelatoRelay internal relay;
    Verifier internal verifier;
    LiquidityManager internal manager;
    zERC20 internal token;
    MintableERC20 internal underlying;
    EndpointV2 internal endpoint;

    uint32 internal constant LOCAL_EID = 1;
    uint32 internal constant HUB_EID = 2;
    uint256 internal constant SIGNER_PK = 0xA11CE;
    address internal signerAddr;

    uint256 internal constant TRANSFER_ROOT = 12345;
    uint64 internal constant ROOT_HINT = 1;

    address internal gelatoRelay;
    address internal feeCollector = address(0xFEE);
    address internal owner = address(0x0ACE);

    bytes32 internal constant RELAYER_FEE_TYPEHASH =
        keccak256("RelayerFeeAuthorization(uint256 recipientHash,uint256 totalValue,uint256 maxFee,uint64 deadline)");

    bytes32 internal constant RELAY_UNWRAP_TYPEHASH = keccak256(
        "RelayUnwrap(address owner,uint256 amount,address receiver,uint256 relayerFee,uint256 maxGelatoFee,uint256 nonce)"
    );

    bytes32 internal constant RELAY_TRANSFER_TYPEHASH = keccak256(
        "RelayTransfer(address owner,address to,uint256 amount,uint256 relayerFee,uint256 maxGelatoFee,uint256 nonce)"
    );

    // solhint-disable-next-line function-max-lines
    function setUp() public override {
        super.setUp();
        setUpEndpoints(2, LibraryType.SimpleMessageLib);
        endpoint = endpointSetup.endpointList[0];
        signerAddr = vm.addr(SIGNER_PK);

        // Deploy underlying ERC20
        underlying = new MintableERC20();

        // Deploy zERC20 token
        zERC20 tokenImpl = new zERC20(address(endpoint), 18);
        bytes memory tokenInit = abi.encodeCall(zERC20.initialize, ("zUSDC", "zUSDC", address(this)));
        token = zERC20(address(new ERC1967Proxy(address(tokenImpl), tokenInit)));

        // Deploy LiquidityManager
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000 ether, k: 0});
        LiquidityManager managerImpl = new LiquidityManager(address(underlying), address(token));
        bytes memory managerInit = abi.encodeCall(LiquidityManager.initialize, (params, address(this)));
        manager = LiquidityManager(payable(address(new ERC1967Proxy(address(managerImpl), managerInit))));
        token.setMinter(address(manager));

        // Seed LiquidityManager with underlying liquidity for unwrap
        underlying.mint(address(manager), 100_000 ether);

        // Deploy Verifier
        MockWithdrawDecider mockDecider = new MockWithdrawDecider();
        MockSingleWithdrawVerifier mockSingleVerifier = new MockSingleWithdrawVerifier();
        Verifier verifierImpl = new Verifier(address(endpoint));
        bytes memory verifierInit = abi.encodeCall(
            Verifier.initialize,
            (
                address(token),
                HUB_EID,
                address(this),
                address(mockDecider),
                address(mockDecider),
                address(mockDecider),
                address(mockSingleVerifier),
                address(mockSingleVerifier)
            )
        );
        verifier = Verifier(address(new ERC1967Proxy(address(verifierImpl), verifierInit)));
        verifier.setPeer(HUB_EID, _toBytes32(address(this)));
        verifier.initializeV2("Verifier", "1");
        token.setVerifier(address(verifier));

        // Store a global root
        Origin memory origin = Origin({srcEid: HUB_EID, sender: _toBytes32(address(this)), nonce: 1});
        bytes memory payload = abi.encode(TRANSFER_ROOT, ROOT_HINT);
        vm.prank(address(endpoint));
        verifier.lzReceive(origin, bytes32(uint256(1)), payload, address(0), bytes(""));

        // Deploy GelatoRelay (impl + proxy)
        GelatoRelay relayImpl = new GelatoRelay(address(verifier), address(manager));
        bytes memory relayInit = abi.encodeCall(GelatoRelay.initialize, (owner, "GelatoRelay", "1"));
        relay = GelatoRelay(payable(address(new ERC1967Proxy(address(relayImpl), relayInit))));

        // Foundry default chainId 31337 maps to GELATO_RELAY_V2
        gelatoRelay = GELATO_RELAY_V2;
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    function _toBytes32(address addr) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(addr)));
    }

    function _buildGr(address recipient) internal view returns (GeneralRecipientLib.GeneralRecipient memory) {
        return GeneralRecipientLib.GeneralRecipient({
            chainId: uint64(block.chainid), recipient: bytes32(uint256(uint160(recipient))), tweak: bytes32(0)
        });
    }

    function _buildNovaProof(uint256 transferRoot, uint256 recipientHash, uint256 totalValue)
        internal
        pure
        returns (bytes memory)
    {
        uint256[34] memory proofArray;
        proofArray[1] = transferRoot;
        proofArray[2] = recipientHash;
        proofArray[3] = 0;
        proofArray[4] = 0;
        proofArray[5] = transferRoot;
        proofArray[6] = recipientHash;
        proofArray[7] = 0;
        proofArray[8] = totalValue;
        return abi.encode(proofArray);
    }

    function _buildSingleProof(uint256 transferRoot, uint256 recipientHash, uint256 totalValue)
        internal
        pure
        returns (bytes memory)
    {
        uint256[2] memory pA;
        uint256[2][2] memory pB;
        uint256[2] memory pC;
        uint256[3] memory pubSignals = [transferRoot, recipientHash, totalValue];
        return abi.encode(pA, pB, pC, pubSignals);
    }

    function _signRelayerFeeAuth(
        uint256 privateKey,
        uint256 recipientHash,
        uint256 totalValue,
        uint256 maxFee,
        uint64 deadline
    ) internal view returns (bytes memory) {
        bytes32 structHash = keccak256(abi.encode(RELAYER_FEE_TYPEHASH, recipientHash, totalValue, maxFee, deadline));
        bytes32 digest = _hashTypedDataV4(structHash);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(privateKey, digest);
        return abi.encodePacked(r, s, v);
    }

    function _hashTypedDataV4(bytes32 structHash) internal view returns (bytes32) {
        (, string memory name_, string memory version_, uint256 chainId_, address verifyingContract_,,) =
            verifier.eip712Domain();
        bytes32 domainSeparator = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256(bytes(name_)),
                keccak256(bytes(version_)),
                chainId_,
                verifyingContract_
            )
        );
        return MessageHashUtils.toTypedDataHash(domainSeparator, structHash);
    }

    function _buildFeeAuth(uint256 relayerFee, uint256 maxFee, uint64 deadline, bytes memory signature)
        internal
        pure
        returns (Verifier.RelayerFeeAuthorization memory)
    {
        return Verifier.RelayerFeeAuthorization({
            relayerFee: relayerFee, maxFee: maxFee, deadline: deadline, signature: signature
        });
    }

    function _signPermit(uint256 amount, uint256 deadline) internal view returns (uint8, bytes32, bytes32) {
        bytes32 permitTypehash =
            keccak256("Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)");
        uint256 nonce = token.nonces(signerAddr);
        bytes32 structHash = keccak256(abi.encode(permitTypehash, signerAddr, address(relay), amount, nonce, deadline));
        bytes32 domainSeparator = token.DOMAIN_SEPARATOR();
        bytes32 digest = MessageHashUtils.toTypedDataHash(domainSeparator, structHash);
        return vm.sign(SIGNER_PK, digest);
    }

    function _relayDomainSeparator() internal view returns (bytes32) {
        (, string memory name_, string memory version_, uint256 chainId_, address verifyingContract_,,) =
            relay.eip712Domain();
        return keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256(bytes(name_)),
                keccak256(bytes(version_)),
                chainId_,
                verifyingContract_
            )
        );
    }

    function _signRelayUnwrap(uint256 amount, address receiver, uint256 relayerFee, uint256 maxGelatoFee)
        internal
        view
        returns (bytes memory)
    {
        uint256 nonce = relay.nonces(signerAddr);
        bytes32 structHash =
            keccak256(abi.encode(RELAY_UNWRAP_TYPEHASH, signerAddr, amount, receiver, relayerFee, maxGelatoFee, nonce));
        bytes32 digest = MessageHashUtils.toTypedDataHash(_relayDomainSeparator(), structHash);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(SIGNER_PK, digest);
        return abi.encodePacked(r, s, v);
    }

    function _signRelayTransfer(address to, uint256 amount, uint256 relayerFee, uint256 maxGelatoFee)
        internal
        view
        returns (bytes memory)
    {
        uint256 nonce = relay.nonces(signerAddr);
        bytes32 structHash =
            keccak256(abi.encode(RELAY_TRANSFER_TYPEHASH, signerAddr, to, amount, relayerFee, maxGelatoFee, nonce));
        bytes32 digest = MessageHashUtils.toTypedDataHash(_relayDomainSeparator(), structHash);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(SIGNER_PK, digest);
        return abi.encodePacked(r, s, v);
    }

    /// @dev Simulates a Gelato relay call by appending relay context to calldata.
    ///      callWithSyncFee appends: abi.encodePacked(_data, _feeCollector, _feeToken, _fee)
    function _callAsGelatoRelay(address target, bytes memory data, address feeToken_, uint256 fee) internal {
        bytes memory fullCalldata = abi.encodePacked(data, feeCollector, feeToken_, fee);
        vm.prank(gelatoRelay);
        // solhint-disable-next-line avoid-low-level-calls
        (bool success, bytes memory ret) = target.call(fullCalldata);
        if (!success) {
            // solhint-disable-next-line no-inline-assembly
            assembly {
                revert(add(ret, 0x20), mload(ret))
            }
        }
    }

    /// @dev Same as _callAsGelatoRelay but expects revert.
    function _callAsGelatoRelayRaw(address target, bytes memory data, address feeToken_, uint256 fee)
        internal
        returns (bool success, bytes memory ret)
    {
        bytes memory fullCalldata = abi.encodePacked(data, feeCollector, feeToken_, fee);
        vm.prank(gelatoRelay);
        // solhint-disable-next-line avoid-low-level-calls
        (success, ret) = target.call(fullCalldata);
    }

    // -----------------------------------------------------------------------
    // Happy path: relayTeleport (Nova)
    // -----------------------------------------------------------------------

    function testRelayTeleportNova() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000 ether;
        uint256 relayerFee = 50 ether;
        uint256 maxFee = 100 ether;
        uint64 deadline = uint64(block.timestamp + 1 hours);
        uint256 gelatoFee = 10 ether; // underlying

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);
        Verifier.RelayerFeeAuthorization memory feeAuth = _buildFeeAuth(relayerFee, maxFee, deadline, signature);

        // Wrap underlying to seed LiquidityManager zERC20 balance awareness
        // LiquidityManager burns zERC20 from msg.sender on unwrap - relay will have zERC20 from teleport

        bytes memory data = abi.encodeCall(GelatoRelay.relayTeleport, (true, ROOT_HINT, gr, proof, feeAuth, gelatoFee));

        uint256 feeCollectorBefore = underlying.balanceOf(feeCollector);
        _callAsGelatoRelay(address(relay), data, address(underlying), gelatoFee);

        // Recipient should have received totalValue - relayerFee in zERC20
        assertEq(token.balanceOf(signerAddr), totalValue - relayerFee, "recipient zERC20 mismatch");

        // Fee collector should have received gelatoFee in underlying
        assertEq(underlying.balanceOf(feeCollector) - feeCollectorBefore, gelatoFee, "feeCollector underlying mismatch");

        // Relay should have surplus underlying (relayerFee unwrapped - gelatoFee paid)
        // With k=0, unwrap fee is 0, so surplus = relayerFee - gelatoFee
        assertEq(underlying.balanceOf(address(relay)), relayerFee - gelatoFee, "relay surplus mismatch");
    }

    // -----------------------------------------------------------------------
    // Happy path: relaySingleTeleport (Groth16)
    // -----------------------------------------------------------------------

    function testRelaySingleTeleportGroth16() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 500 ether;
        uint256 relayerFee = 25 ether;
        uint256 maxFee = 50 ether;
        uint64 deadline = uint64(block.timestamp + 1 hours);
        uint256 gelatoFee = 5 ether;

        bytes memory proof = _buildSingleProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);
        Verifier.RelayerFeeAuthorization memory feeAuth = _buildFeeAuth(relayerFee, maxFee, deadline, signature);

        bytes memory data =
            abi.encodeCall(GelatoRelay.relaySingleTeleport, (true, ROOT_HINT, gr, proof, feeAuth, gelatoFee));

        _callAsGelatoRelay(address(relay), data, address(underlying), gelatoFee);

        assertEq(token.balanceOf(signerAddr), totalValue - relayerFee, "recipient zERC20 mismatch");
        assertEq(underlying.balanceOf(feeCollector), gelatoFee, "feeCollector underlying mismatch");
    }

    // -----------------------------------------------------------------------
    // onlyGelatoRelay: non-relay caller reverts
    // -----------------------------------------------------------------------

    function testRevertWhenNotGelatoRelay() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000 ether;
        uint256 relayerFee = 50 ether;
        uint256 maxFee = 100 ether;
        uint64 deadline = uint64(block.timestamp + 1 hours);

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);
        Verifier.RelayerFeeAuthorization memory feeAuth = _buildFeeAuth(relayerFee, maxFee, deadline, signature);

        vm.expectRevert(GelatoRelay.OnlyGelatoRelay.selector);
        relay.relayTeleport(true, ROOT_HINT, gr, proof, feeAuth, 10 ether);
    }

    // -----------------------------------------------------------------------
    // maxGelatoFee: Gelato fee exceeds max
    // -----------------------------------------------------------------------

    function testRevertWhenGelatoFeeExceedsMax() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000 ether;
        uint256 relayerFee = 50 ether;
        uint256 maxFee = 100 ether;
        uint64 deadline = uint64(block.timestamp + 1 hours);
        uint256 maxGelatoFee = 5 ether;
        uint256 actualGelatoFee = 10 ether; // exceeds maxGelatoFee

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);
        Verifier.RelayerFeeAuthorization memory feeAuth = _buildFeeAuth(relayerFee, maxFee, deadline, signature);

        bytes memory data =
            abi.encodeCall(GelatoRelay.relayTeleport, (true, ROOT_HINT, gr, proof, feeAuth, maxGelatoFee));

        (bool success,) = _callAsGelatoRelayRaw(address(relay), data, address(underlying), actualGelatoFee);
        assertFalse(success, "should revert when gelato fee exceeds max");
    }

    // -----------------------------------------------------------------------
    // relayerFee == 0: no unwrap, still pays Gelato from existing balance
    // -----------------------------------------------------------------------

    function testRelayerFeeZeroSkipsUnwrap() public {
        GeneralRecipientLib.GeneralRecipient memory gr = _buildGr(signerAddr);
        uint256 recipientHash = GeneralRecipientLib.hash(gr);
        uint256 totalValue = 1000 ether;
        uint256 relayerFee = 0;
        uint256 maxFee = 0;
        uint64 deadline = uint64(block.timestamp + 1 hours);
        uint256 gelatoFee = 5 ether;

        bytes memory proof = _buildNovaProof(TRANSFER_ROOT, recipientHash, totalValue);
        bytes memory signature = _signRelayerFeeAuth(SIGNER_PK, recipientHash, totalValue, maxFee, deadline);
        Verifier.RelayerFeeAuthorization memory feeAuth = _buildFeeAuth(relayerFee, maxFee, deadline, signature);

        // Pre-fund relay with underlying so it can pay Gelato even with relayerFee=0
        underlying.mint(address(relay), gelatoFee);

        bytes memory data = abi.encodeCall(GelatoRelay.relayTeleport, (true, ROOT_HINT, gr, proof, feeAuth, gelatoFee));

        _callAsGelatoRelay(address(relay), data, address(underlying), gelatoFee);

        // Recipient gets full totalValue
        assertEq(token.balanceOf(signerAddr), totalValue, "recipient should get full value");
        // Fee collector gets paid
        assertEq(underlying.balanceOf(feeCollector), gelatoFee, "feeCollector should be paid");
    }

    // -----------------------------------------------------------------------
    // withdrawSurplus: owner withdraws ERC20 surplus
    // -----------------------------------------------------------------------

    function testWithdrawSurplus() public {
        underlying.mint(address(relay), 100 ether);

        vm.prank(owner);
        relay.withdrawSurplus(address(underlying), owner, 100 ether);

        assertEq(underlying.balanceOf(owner), 100 ether, "owner should receive surplus");
        assertEq(underlying.balanceOf(address(relay)), 0, "relay should have 0 balance");
    }

    function testWithdrawSurplusRevertsForNonOwner() public {
        underlying.mint(address(relay), 100 ether);

        vm.prank(address(0xBAD));
        vm.expectRevert();
        relay.withdrawSurplus(address(underlying), address(0xBAD), 100 ether);
    }

    // -----------------------------------------------------------------------
    // withdrawSurplusNative: owner withdraws native surplus
    // -----------------------------------------------------------------------

    function testWithdrawSurplusNative() public {
        vm.deal(address(relay), 10 ether);

        vm.prank(owner);
        relay.withdrawSurplusNative(payable(owner), 10 ether);

        assertEq(owner.balance, 10 ether, "owner should receive native surplus");
    }

    function testWithdrawSurplusNativeRevertsForNonOwner() public {
        vm.deal(address(relay), 10 ether);

        vm.prank(address(0xBAD));
        vm.expectRevert();
        relay.withdrawSurplusNative(payable(address(0xBAD)), 10 ether);
    }

    // -----------------------------------------------------------------------
    // Constructor validation
    // -----------------------------------------------------------------------

    function testConstructorRevertsOnZeroVerifier() public {
        vm.expectRevert(GelatoRelay.ZeroAddress.selector);
        new GelatoRelay(address(0), address(manager));
    }

    function testConstructorRevertsOnZeroLiquidityManager() public {
        vm.expectRevert(GelatoRelay.ZeroAddress.selector);
        new GelatoRelay(address(verifier), address(0));
    }

    // -----------------------------------------------------------------------
    // Initialize / Upgrade
    // -----------------------------------------------------------------------

    function testInitialize() public view {
        assertEq(relay.owner(), owner, "owner mismatch");
    }

    function testCannotReinitialize() public {
        vm.expectRevert();
        relay.initialize(address(0xBEEF), "GelatoRelay", "1");
    }

    function testAuthorizeUpgradeRevertsOnVerifierMismatch() public {
        // Deploy a new impl with a different verifier
        Verifier otherVerifierImpl = new Verifier(address(endpoint));
        bytes memory otherVerifierInit = abi.encodeCall(
            Verifier.initialize,
            (
                address(token),
                HUB_EID,
                address(this),
                address(new MockWithdrawDecider()),
                address(new MockWithdrawDecider()),
                address(new MockWithdrawDecider()),
                address(new MockSingleWithdrawVerifier()),
                address(new MockSingleWithdrawVerifier())
            )
        );
        Verifier otherVerifier = Verifier(address(new ERC1967Proxy(address(otherVerifierImpl), otherVerifierInit)));

        GelatoRelay badImpl = new GelatoRelay(address(otherVerifier), address(manager));

        vm.prank(owner);
        vm.expectRevert(
            abi.encodeWithSelector(GelatoRelay.VerifierMismatch.selector, address(verifier), address(otherVerifier))
        );
        relay.upgradeToAndCall(address(badImpl), "");
    }

    function testAuthorizeUpgradeRevertsOnLiquidityManagerMismatch() public {
        // Deploy a new LiquidityManager with different address
        IncentiveLib.FeeParams memory params = IncentiveLib.FeeParams({targetLiquidity: 1_000 ether, k: 0});
        LiquidityManager otherManagerImpl = new LiquidityManager(address(underlying), address(token));
        bytes memory otherManagerInit = abi.encodeCall(LiquidityManager.initialize, (params, address(this)));
        LiquidityManager otherManager =
            LiquidityManager(payable(address(new ERC1967Proxy(address(otherManagerImpl), otherManagerInit))));

        GelatoRelay badImpl = new GelatoRelay(address(verifier), address(otherManager));

        vm.prank(owner);
        vm.expectRevert(
            abi.encodeWithSelector(
                GelatoRelay.LiquidityManagerMismatch.selector, address(manager), address(otherManager)
            )
        );
        relay.upgradeToAndCall(address(badImpl), "");
    }

    function testAuthorizeUpgradeSucceedsWithMatchingImmutables() public {
        GelatoRelay newImpl = new GelatoRelay(address(verifier), address(manager));

        vm.prank(owner);
        relay.upgradeToAndCall(address(newImpl), "");
    }

    // -----------------------------------------------------------------------
    // Native underlying (ETH/BNB) flow
    // -----------------------------------------------------------------------

    function testRelayTeleportNativeUnderlying() public {
        // Verify the relay's receive() accepts native tokens (ETH/BNB from LiquidityManager.unwrap)
        vm.deal(address(relay), 10 ether);
        assertGt(address(relay).balance, 0, "relay should accept native tokens");
    }

    // -----------------------------------------------------------------------
    // Immutable getters
    // -----------------------------------------------------------------------

    function testImmutableGetters() public view {
        assertEq(address(relay.VERIFIER()), address(verifier), "VERIFIER mismatch");
        assertEq(address(relay.LIQUIDITY_MANAGER()), address(manager), "LIQUIDITY_MANAGER mismatch");
        assertEq(address(relay.UNDERLYING_TOKEN()), address(underlying), "UNDERLYING_TOKEN mismatch");
        assertEq(address(relay.ZERC20_TOKEN()), address(token), "ZERC20_TOKEN mismatch");
    }

    // -----------------------------------------------------------------------
    // relayUnwrap: permit → unwrap → underlying to receiver
    // -----------------------------------------------------------------------

    function testRelayUnwrap() public {
        uint256 amount = 100 ether;
        uint256 relayerFee = 10 ether;
        uint256 gelatoFee = 5 ether;
        address receiver = address(0xBEEF);

        // Mint zERC20 to signer via manager (wrap underlying)
        underlying.mint(address(this), amount + relayerFee);
        underlying.approve(address(manager), amount + relayerFee);
        manager.wrap(amount + relayerFee, signerAddr);

        bytes memory data = _buildRelayUnwrapData(amount, relayerFee, receiver, gelatoFee);

        _callAsGelatoRelay(address(relay), data, address(underlying), gelatoFee);

        // Signer should have no zERC20 left
        assertEq(token.balanceOf(signerAddr), 0, "signer zERC20 should be 0");
        // Receiver should have underlying from the user-requested unwrap only
        assertEq(underlying.balanceOf(receiver), amount, "receiver underlying mismatch");
        // Fee collector should have received gelatoFee
        assertEq(underlying.balanceOf(feeCollector), gelatoFee, "feeCollector underlying mismatch");
        // Relay keeps surplus underlying (relayerFee unwrapped - gelatoFee)
        assertEq(underlying.balanceOf(address(relay)), relayerFee - gelatoFee, "relay surplus mismatch");
    }

    function _buildRelayUnwrapData(uint256 amount, uint256 relayerFee, address receiver, uint256 gelatoFee)
        internal
        view
        returns (bytes memory)
    {
        uint256 deadline = block.timestamp + 1 hours;
        (uint8 v, bytes32 r, bytes32 s) = _signPermit(amount + relayerFee, deadline);
        bytes memory permitSig = abi.encodePacked(r, s, v);
        bytes memory relaySig = _signRelayUnwrap(amount, receiver, relayerFee, gelatoFee);
        return abi.encodeCall(
            GelatoRelay.relayUnwrap,
            (signerAddr, amount, receiver, relayerFee, gelatoFee, deadline, permitSig, relaySig)
        );
    }

    function testRelayUnwrapRevertsWhenNotGelatoRelay() public {
        vm.expectRevert(GelatoRelay.OnlyGelatoRelay.selector);
        relay.relayUnwrap(signerAddr, 100 ether, address(0xBEEF), 10 ether, 5 ether, block.timestamp + 1 hours, "", "");
    }

    // -----------------------------------------------------------------------
    // relayTransfer: permit → transfer zERC20 → unwrap relayerFee
    // -----------------------------------------------------------------------

    function testRelayTransfer() public {
        uint256 amount = 100 ether;
        uint256 relayerFee = 10 ether;
        uint256 gelatoFee = 5 ether;
        address recipient = address(0xCAFE);

        // Mint zERC20 to signer
        underlying.mint(address(this), amount);
        underlying.approve(address(manager), amount);
        manager.wrap(amount, signerAddr);

        bytes memory data = _buildRelayTransferData(amount, relayerFee, recipient, gelatoFee);

        _callAsGelatoRelay(address(relay), data, address(underlying), gelatoFee);

        // Signer should have no zERC20 left
        assertEq(token.balanceOf(signerAddr), 0, "signer zERC20 should be 0");
        // Recipient should have (amount - relayerFee) zERC20
        assertEq(token.balanceOf(recipient), amount - relayerFee, "recipient zERC20 mismatch");
        // Fee collector should have received gelatoFee in underlying
        assertEq(underlying.balanceOf(feeCollector), gelatoFee, "feeCollector underlying mismatch");
        // Relay keeps surplus underlying (relayerFee unwrapped - gelatoFee)
        assertEq(underlying.balanceOf(address(relay)), relayerFee - gelatoFee, "relay surplus mismatch");
    }

    function _buildRelayTransferData(uint256 amount, uint256 relayerFee, address recipient, uint256 gelatoFee)
        internal
        view
        returns (bytes memory)
    {
        uint256 deadline = block.timestamp + 1 hours;
        (uint8 v, bytes32 r, bytes32 s) = _signPermit(amount, deadline);
        bytes memory permitSig = abi.encodePacked(r, s, v);
        bytes memory relaySig = _signRelayTransfer(recipient, amount, relayerFee, gelatoFee);
        return abi.encodeCall(
            GelatoRelay.relayTransfer,
            (signerAddr, recipient, amount, relayerFee, gelatoFee, deadline, permitSig, relaySig)
        );
    }

    function testRelayTransferRevertsWhenNotGelatoRelay() public {
        vm.expectRevert(GelatoRelay.OnlyGelatoRelay.selector);
        relay.relayTransfer(
            signerAddr, address(0xCAFE), 100 ether, 10 ether, 5 ether, block.timestamp + 1 hours, "", ""
        );
    }

    // -----------------------------------------------------------------------
    // Replay protection: relayUnwrap nonce prevents replay
    // -----------------------------------------------------------------------

    function testRelayUnwrapReplayReverts() public {
        uint256 amount = 100 ether;
        uint256 relayerFee = 10 ether;
        uint256 gelatoFee = 5 ether;
        address receiver = address(0xBEEF);

        // Mint enough for two attempts
        underlying.mint(address(this), 2 * (amount + relayerFee));
        underlying.approve(address(manager), 2 * (amount + relayerFee));
        manager.wrap(amount + relayerFee, signerAddr);

        bytes memory data = _buildRelayUnwrapData(amount, relayerFee, receiver, gelatoFee);

        // First call succeeds
        _callAsGelatoRelay(address(relay), data, address(underlying), gelatoFee);

        // Mint more zERC20 for the second attempt
        manager.wrap(amount + relayerFee, signerAddr);

        // Replay with the same calldata should fail (nonce consumed)
        (bool success,) = _callAsGelatoRelayRaw(address(relay), data, address(underlying), gelatoFee);
        assertFalse(success, "replay relayUnwrap should revert");
    }

    // -----------------------------------------------------------------------
    // Replay protection: relayTransfer nonce prevents replay
    // -----------------------------------------------------------------------

    function testRelayTransferReplayReverts() public {
        uint256 amount = 100 ether;
        uint256 relayerFee = 10 ether;
        uint256 gelatoFee = 5 ether;
        address recipient = address(0xCAFE);

        // Mint enough for two attempts
        underlying.mint(address(this), 2 * amount);
        underlying.approve(address(manager), 2 * amount);
        manager.wrap(amount, signerAddr);

        bytes memory data = _buildRelayTransferData(amount, relayerFee, recipient, gelatoFee);

        // First call succeeds
        _callAsGelatoRelay(address(relay), data, address(underlying), gelatoFee);

        // Mint more zERC20 for the second attempt
        manager.wrap(amount, signerAddr);

        // Replay with the same calldata should fail (nonce consumed)
        (bool success,) = _callAsGelatoRelayRaw(address(relay), data, address(underlying), gelatoFee);
        assertFalse(success, "replay relayTransfer should revert");
    }

    // -----------------------------------------------------------------------
    // Wrong signer: relayUnwrap with wrong private key reverts
    // -----------------------------------------------------------------------

    function testRelayUnwrapWrongSignerReverts() public {
        uint256 amount = 100 ether;
        uint256 relayerFee = 10 ether;
        uint256 gelatoFee = 5 ether;
        address receiver = address(0xBEEF);

        underlying.mint(address(this), amount + relayerFee);
        underlying.approve(address(manager), amount + relayerFee);
        manager.wrap(amount + relayerFee, signerAddr);

        // Build calldata with wrong-key relay signature
        bytes memory data =
            _buildRelayUnwrapDataWrongSigner(amount, relayerFee, receiver, gelatoFee, block.timestamp + 1 hours);

        (bool success,) = _callAsGelatoRelayRaw(address(relay), data, address(underlying), gelatoFee);
        assertFalse(success, "wrong signer relayUnwrap should revert");
    }

    function _buildRelayUnwrapDataWrongSigner(
        uint256 amount,
        uint256 relayerFee,
        address receiver,
        uint256 gelatoFee,
        uint256 deadline
    ) internal view returns (bytes memory) {
        bytes memory permitSig;
        {
            (uint8 v, bytes32 r, bytes32 s) = _signPermit(amount + relayerFee, deadline);
            permitSig = abi.encodePacked(r, s, v);
        }

        bytes memory relaySig;
        {
            bytes32 structHash = keccak256(
                abi.encode(
                    RELAY_UNWRAP_TYPEHASH, signerAddr, amount, receiver, relayerFee, gelatoFee, relay.nonces(signerAddr)
                )
            );
            bytes32 digest = MessageHashUtils.toTypedDataHash(_relayDomainSeparator(), structHash);
            (uint8 v2, bytes32 r2, bytes32 s2) = vm.sign(0xDEAD, digest);
            relaySig = abi.encodePacked(r2, s2, v2);
        }

        return abi.encodeCall(
            GelatoRelay.relayUnwrap,
            (signerAddr, amount, receiver, relayerFee, gelatoFee, deadline, permitSig, relaySig)
        );
    }

    // -----------------------------------------------------------------------
    // Wrong signer: relayTransfer with wrong private key reverts
    // -----------------------------------------------------------------------

    function testRelayTransferWrongSignerReverts() public {
        uint256 amount = 100 ether;
        uint256 relayerFee = 10 ether;
        uint256 gelatoFee = 5 ether;
        address recipient = address(0xCAFE);

        underlying.mint(address(this), amount);
        underlying.approve(address(manager), amount);
        manager.wrap(amount, signerAddr);

        // Build calldata with wrong-key relay signature
        bytes memory data =
            _buildRelayTransferDataWrongSigner(amount, relayerFee, recipient, gelatoFee, block.timestamp + 1 hours);

        (bool success,) = _callAsGelatoRelayRaw(address(relay), data, address(underlying), gelatoFee);
        assertFalse(success, "wrong signer relayTransfer should revert");
    }

    function _buildRelayTransferDataWrongSigner(
        uint256 amount,
        uint256 relayerFee,
        address recipient,
        uint256 gelatoFee,
        uint256 deadline
    ) internal view returns (bytes memory) {
        bytes memory permitSig;
        {
            (uint8 v, bytes32 r, bytes32 s) = _signPermit(amount, deadline);
            permitSig = abi.encodePacked(r, s, v);
        }

        bytes memory relaySig;
        {
            bytes32 structHash = keccak256(
                abi.encode(
                    RELAY_TRANSFER_TYPEHASH,
                    signerAddr,
                    recipient,
                    amount,
                    relayerFee,
                    gelatoFee,
                    relay.nonces(signerAddr)
                )
            );
            bytes32 digest = MessageHashUtils.toTypedDataHash(_relayDomainSeparator(), structHash);
            (uint8 v2, bytes32 r2, bytes32 s2) = vm.sign(0xDEAD, digest);
            relaySig = abi.encodePacked(r2, s2, v2);
        }

        return abi.encodeCall(
            GelatoRelay.relayTransfer,
            (signerAddr, recipient, amount, relayerFee, gelatoFee, deadline, permitSig, relaySig)
        );
    }
}
