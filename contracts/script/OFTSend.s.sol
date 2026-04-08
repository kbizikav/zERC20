// SPDX-License-Identifier: BUSL-1.1
pragma solidity 0.8.33;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";

import {IzERC20} from "../src/interfaces/IzERC20.sol";
import {SendParam, MessagingFee, OFTReceipt} from "@layerzerolabs/oft-evm/contracts/interfaces/IOFT.sol";
import {OptionsBuilder} from "@layerzerolabs/oapp-evm/contracts/oapp/libs/OptionsBuilder.sol";

/// @notice Sends zERC20 as an OFT to the destination EID.
/// Env:
/// - ZERC20 (address)     : zERC20 token address on the source chain.
/// - TO_EID (uint256)     : Destination LayerZero endpoint ID.
/// - AMOUNT (uint256)     : Amount of zERC20 to send (local decimals).
/// - PRIVATE_KEY (uint256): Broadcaster key that owns the tokens.
contract OFTSend is Script {
    using OptionsBuilder for bytes;

    uint128 private constant LZ_RECEIVE_GAS = 500_000;

    struct Inputs {
        address zerc20;
        uint32 toEid;
        uint256 amount;
        uint256 broadcasterKey;
        address broadcaster;
    }

    error TokenMissing();
    error AmountMissing();
    error EidOverflow(uint256 value);

    function run() external {
        Inputs memory inputs = _loadInputs();
        IzERC20 token = IzERC20(inputs.zerc20);

        bytes memory extraOptions = OptionsBuilder.newOptions().addExecutorLzReceiveOption(LZ_RECEIVE_GAS, 0);
        SendParam memory sendParam = SendParam({
            dstEid: inputs.toEid,
            to: _addressToBytes32(inputs.broadcaster),
            amountLD: inputs.amount,
            minAmountLD: inputs.amount,
            extraOptions: extraOptions,
            composeMsg: bytes(""),
            oftCmd: bytes("")
        });

        MessagingFee memory fee = token.quoteSend(sendParam, false);

        console2.log("Sending zERC20 as OFT");
        console2.log("  token", inputs.zerc20);
        console2.log("  to eid", uint256(inputs.toEid));
        console2.log("  receiver", inputs.broadcaster);
        console2.log("  amount", inputs.amount);
        console2.log("  native fee", fee.nativeFee);

        vm.startBroadcast(inputs.broadcasterKey);
        // solhint-disable-next-line check-send-result
        (, OFTReceipt memory receipt) = token.send{value: fee.nativeFee}(sendParam, fee, inputs.broadcaster);
        vm.stopBroadcast();

        console2.log("OFT send complete");
        console2.log("  amount sent", receipt.amountSentLD);
        console2.log("  amount received", receipt.amountReceivedLD);
    }

    function _loadInputs() internal view returns (Inputs memory inputs) {
        inputs.zerc20 = vm.envAddress("ZERC20");
        if (inputs.zerc20 == address(0)) revert TokenMissing();

        inputs.amount = vm.envUint("AMOUNT");
        if (inputs.amount == 0) revert AmountMissing();

        inputs.toEid = _toUint32(vm.envUint("TO_EID"));

        inputs.broadcasterKey = vm.envUint("PRIVATE_KEY");
        inputs.broadcaster = vm.addr(inputs.broadcasterKey);
    }

    function _addressToBytes32(address addr) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(addr)));
    }

    function _toUint32(uint256 value) internal pure returns (uint32) {
        if (value > type(uint32).max) {
            revert EidOverflow(value);
        }
        // casting to uint32 is safe because we revert on overflow above
        // forge-lint: disable-next-line(unsafe-typecast)
        return uint32(value);
    }
}
