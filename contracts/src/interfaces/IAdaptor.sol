// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IAdaptor {
    /// @notice Breaks down the expected fees for unwrapping and bridging.
    struct FeeQuote {
        uint256 tokenUnwrapFee;
        uint256 nativeBridgeFee;
        uint256 tokenBridgeFee;
    }

    /// @notice Parameters for forwarding unwrapped tokens through Stargate.
    struct BridgeRequest {
        uint32 dstEid;
        address to;
        uint256 minAmountOut; // Minimum bridged tokens expected on the destination chain.
        bytes extraOptions;
        bytes composeMsg;
        bytes oftCmd;
    }
    // forge-lint: disable-next-line(mixed-case-function)
    function LIQUIDITY_MANAGER() external returns (address);
    // forge-lint: disable-next-line(mixed-case-function)
    function STARGATE() external returns (address);
    // forge-lint: disable-next-line(mixed-case-function)
    function LZ_ENDPOINT() external returns (address);
    // forge-lint: disable-next-line(mixed-case-function)
    function UNDERLYING_TOKEN() external returns (address);
    // forge-lint: disable-next-line(mixed-case-function)
    function ZERC20_TOKEN() external returns (address);

    function underlyingTokenBalances(address user) external view returns (uint256);
    function zerc20Balances(address user) external view returns (uint256);
    function nativeBalances(address user) external view returns (uint256);
    function unwrapAndBridge(uint256 zerc20Amount, BridgeRequest calldata request) external payable;
    function withdraw(address token, uint256 amount) external;
    function quoteFee(uint256 amount, BridgeRequest memory request) external view returns (FeeQuote memory quote);
    function decodeBridgeRequest(bytes calldata _message) external pure returns (BridgeRequest memory request);
    function unwrapSelf(address user, uint256 amount, uint256 amountMinOut) external returns (uint256 amountOut);
    function bridgeUnderlyingTokenSelf(
        address user,
        uint256 amount,
        uint256 nativeBridgeFee,
        BridgeRequest calldata request
    ) external returns (uint256 amountOut);
    function bridgeZerc20Self(uint32 dstEid, address user, address to, uint256 amount) external;
}
