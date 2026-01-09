// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IMintableBurnableERC20} from "../src/interfaces/IMintableBurnableERC20.sol";
import {ILiquidityManager} from "../src/interfaces/ILiquidityManager.sol";

/// @notice Mints the chain-configured underlying token, approves the LiquidityManager, and wraps it into zERC20.
/// Env:
/// - PRIVATE_KEY (uint256)          : Broadcaster key that receives the minted tokens.
/// - LIQUIDITY_MANAGER (address)    : LiquidityManager to approve and wrap into.
/// - WRAP_AMOUNT (uint256)          : Amount of underlying (base units) to mint and wrap.
/// - WRAP_RECEIVER (address)        : Optional zERC20 recipient (defaults to broadcaster).
/// - UNDERLYING_TOKEN (address)     : Optional override; otherwise read from chain-config.
/// - CHAIN_CONFIG_PATH (string)     : Optional path to chain-config JSON (defaults to config/chain-config.json).
contract MintApproveAndWrap is Script {
    string internal constant DEFAULT_CHAIN_CONFIG_PATH = "config/chain-config.json";
    address internal constant NATIVE_TOKEN = 0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE;

    struct Inputs {
        address underlyingToken;
        address liquidityManager;
        address wrapReceiver;
        uint256 amount;
        uint256 broadcasterKey;
        address broadcaster;
    }

    error UnderlyingTokenMissing(uint256 chainId);
    error LiquidityManagerMissing();
    error AmountMissing();

    function run() external {
        Inputs memory inputs = _loadInputs();

        vm.startBroadcast(inputs.broadcasterKey);

        bool isNative = inputs.underlyingToken == NATIVE_TOKEN;

        if (isNative) {
            console2.log("Using native token for wrap");
        } else {
            console2.log("Minting underlying tokens");
            console2.log("  token", inputs.underlyingToken);
            console2.log("  to", inputs.broadcaster);
            console2.log("  amount", inputs.amount);
            IMintableBurnableERC20(inputs.underlyingToken).mint(inputs.broadcaster, inputs.amount);

            console2.log("Approving LiquidityManager", inputs.liquidityManager);
            bool approved = IERC20(inputs.underlyingToken).approve(inputs.liquidityManager, inputs.amount);
            require(approved, "approve failed");
        }

        console2.log("Wrapping and sending to", inputs.wrapReceiver);
        uint256 amountOut;
        if (isNative) {
            amountOut = ILiquidityManager(inputs.liquidityManager).wrap{value: inputs.amount}(
                inputs.amount, inputs.wrapReceiver
            );
        } else {
            amountOut = ILiquidityManager(inputs.liquidityManager).wrap(inputs.amount, inputs.wrapReceiver);
        }
        console2.log("zERC20 minted", amountOut);

        vm.stopBroadcast();
    }

    function _loadInputs() internal view returns (Inputs memory inputs) {
        inputs.amount = vm.envUint("WRAP_AMOUNT");
        if (inputs.amount == 0) revert AmountMissing();

        inputs.liquidityManager = vm.envAddress("LIQUIDITY_MANAGER");
        if (inputs.liquidityManager == address(0)) revert LiquidityManagerMissing();

        inputs.broadcasterKey = vm.envUint("PRIVATE_KEY");
        inputs.broadcaster = vm.addr(inputs.broadcasterKey);
        inputs.wrapReceiver = vm.envOr("WRAP_RECEIVER", inputs.broadcaster);

        inputs.underlyingToken = vm.envOr("UNDERLYING_TOKEN", _loadUnderlyingFromChainConfig());
        if (inputs.underlyingToken == address(0)) revert UnderlyingTokenMissing(block.chainid);
    }

    function _loadUnderlyingFromChainConfig() internal view returns (address underlying) {
        string memory path = vm.envOr("CHAIN_CONFIG_PATH", DEFAULT_CHAIN_CONFIG_PATH);

        string memory json;
        // reading local config files is intentional in scripts
        // forge-lint: disable-next-line(unsafe-cheatcode)
        try vm.readFile(path) returns (string memory data) {
            json = data;
        } catch (bytes memory) {
            return address(0);
        }

        string memory base = string.concat(".chains[\"", _toString(block.chainid), "\"]");
        underlying = _parseAddress(json, string.concat(base, ".underlyingToken"));
    }

    function _parseAddress(string memory json, string memory key) private pure returns (address value) {
        try vm.parseJsonAddress(json, key) returns (address parsed) {
            value = parsed;
        } catch (bytes memory) {
            value = address(0);
        }
    }

    function _toString(uint256 value) internal pure returns (string memory) {
        if (value == 0) {
            return "0";
        }

        uint256 temp = value;
        uint256 digits;
        while (temp != 0) {
            digits++;
            temp /= 10;
        }

        bytes memory buffer = new bytes(digits);
        while (value != 0) {
            digits -= 1;
            buffer[digits] = bytes1(uint8(48 + (value % 10)));
            value /= 10;
        }

        return string(buffer);
    }
}
