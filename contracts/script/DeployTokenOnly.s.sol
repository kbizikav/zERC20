// SPDX-License-Identifier: MIT
pragma solidity 0.8.33;

import {console2} from "forge-std/console2.sol";
import {zERC20} from "../src/zERC20.sol";
import {DeterministicDeployer} from "./utils/DeterministicDeploy.sol";

/// @notice Deploys only the zERC20 token (implementation + ERC1967Proxy) and runs initialize + setMinter.
/// @dev Defaults are set for Base mainnet; override via env vars if needed.
/// - PRIVATE_KEY (uint256)   : broadcaster private key (must be the owner if SET_MINTER=1)
/// - TOKEN_NAME (string)     : defaults to "new_ITX"
/// - TOKEN_SYMBOL (string)   : defaults to "nITX"
/// - TOKEN_OWNER (address)   : defaults to 0x18DE9A6028cFAa0B4B58cc72E257b12e5625B396
/// - ENDPOINT (address)      : defaults to 0x1a44076050125825900e736c501f859c50fE728c
/// - TOKEN_DECIMALS (uint256): defaults to 18
/// - MINTER (address)        : defaults to TOKEN_OWNER
/// - SET_MINTER (uint256)    : defaults to 1 (set to 0 to skip setMinter)
/// - DEPLOY_SALT (string)    : optional; to avoid CREATE3 address collisions
contract DeployTokenOnly is DeterministicDeployer {
    error WrongChainId(uint256 expected, uint256 actual);
    error MissingEnv(string key);
    error MustBroadcastFromOwner(address broadcaster, address owner);

    function run() external {
        _enforceBaseMainnet();

        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);

        string memory name_ = vm.envOr("TOKEN_NAME", string("new_ITX"));
        string memory symbol_ = vm.envOr("TOKEN_SYMBOL", string("nITX"));
        address owner_ = vm.envOr("TOKEN_OWNER", address(0x18DE9A6028cFAa0B4B58cc72E257b12e5625B396));
        address endpoint_ = vm.envOr("ENDPOINT", address(0x1a44076050125825900e736c501f859c50fE728c));
        uint256 decimalsRaw = vm.envOr("TOKEN_DECIMALS", uint256(18));
        address minter_ = vm.envOr("MINTER", owner_);
        uint256 setMinterRaw = vm.envOr("SET_MINTER", uint256(1));

        if (bytes(name_).length == 0) revert MissingEnv("TOKEN_NAME");
        if (bytes(symbol_).length == 0) revert MissingEnv("TOKEN_SYMBOL");
        if (owner_ == address(0)) revert MissingEnv("TOKEN_OWNER");
        if (endpoint_ == address(0)) revert MissingEnv("ENDPOINT");
        if (decimalsRaw > type(uint8).max) revert MissingEnv("TOKEN_DECIMALS");
        // matches other scripts' constraints
        require(decimalsRaw >= 6, "decimals must be >= 6");

        // forge-lint: disable-next-line(unsafe-typecast)
        uint8 decimals_ = uint8(decimalsRaw);
        bool setMinter_ = setMinterRaw != 0;

        if (setMinter_ && deployer != owner_) {
            revert MustBroadcastFromOwner(deployer, owner_);
        }

        bytes32 baseSalt = _loadBaseSalt();

        vm.startBroadcast(deployerKey);
        console2.log("Deploying zERC20 (token only) at block", block.number);
        console2.log("  broadcaster", deployer);

        bytes memory tokenImplCode = abi.encodePacked(type(zERC20).creationCode, abi.encode(endpoint_, decimals_));
        zERC20 tokenImpl = zERC20(_deploy3(baseSalt, "TOKEN_IMPL", tokenImplCode));

        bytes memory tokenInit = abi.encodeCall(zERC20.initialize, (name_, symbol_, owner_));
        zERC20 token = zERC20(_deployProxyAndInit(baseSalt, "TOKEN_PROXY", address(tokenImpl), tokenInit));

        console2.log("Token implementation deployed at", address(tokenImpl));
        console2.log("Token proxy deployed at", address(token));
        console2.log("  endpoint", endpoint_);
        console2.log("  decimals", uint256(decimals_));
        console2.log("  owner", owner_);

        if (setMinter_) {
            token.setMinter(minter_);
            console2.log("  minter set to", minter_);
        } else {
            console2.log("  setMinter skipped (SET_MINTER=0)");
        }

        vm.stopBroadcast();
    }

    function _enforceBaseMainnet() private view {
        uint256 expected = 8453;
        if (block.chainid != expected) revert WrongChainId(expected, block.chainid);
    }
}

