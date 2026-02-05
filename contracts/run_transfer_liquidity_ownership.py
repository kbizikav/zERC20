#!/usr/bin/env python3
"""Run TransferLiquidityOwnership.s.sol for all LiquidityManagers and Adaptors in tokens.json."""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Sequence


SCRIPT_DIR = Path(__file__).resolve().parent
ROOT_DIR = SCRIPT_DIR.parent
DEFAULT_TOKENS_FILE = (ROOT_DIR / "config" / "tokens.json").resolve()
TRANSFER_LM_OWNERSHIP_SCRIPT = "script/TransferLiquidityOwnership.s.sol:TransferLiquidityManagerOwnership"
TRANSFER_ADAPTOR_OWNERSHIP_SCRIPT = "script/TransferLiquidityOwnership.s.sol:TransferAdaptorOwnership"


class ConfigError(RuntimeError):
    """Raised when the configuration is invalid or missing required fields."""


@dataclass
class ContractConfig:
    label: str
    contract_address: str
    contract_type: str  # "liquidity_manager" or "adaptor"
    chain_id: str
    rpc_url: str
    legacy_tx: bool


def parse_args() -> tuple[Path, str, str, list[str]]:
    parser = argparse.ArgumentParser(
        usage="run_transfer_liquidity_ownership.py [--file PATH] --type TYPE [--] [forge flags...]",
        description=(
            "Reads a tokens.json-formatted file and runs TransferLiquidityOwnership.s.sol "
            "for each LiquidityManager or Adaptor to transfer ownership to NEW_OWNER."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Examples:\n"
            "  NEW_OWNER=0x123... ./run_transfer_liquidity_ownership.py --type liquidity_manager\n"
            "  NEW_OWNER=0x123... ./run_transfer_liquidity_ownership.py --type adaptor\n"
            "  NEW_OWNER=0x123... ./run_transfer_liquidity_ownership.py --type all\n"
            "  NEW_OWNER=0x123... ./run_transfer_liquidity_ownership.py --type all --file ../config/tokens.prod.json -- --broadcast -vv\n"
        ),
    )
    parser.add_argument("--file", dest="tokens_file", help="Path to tokens.json (defaults to ../config/tokens.json)")
    parser.add_argument("positional_file", nargs="?", help="Optional tokens.json path when not using --file")
    parser.add_argument(
        "--type",
        choices=["liquidity_manager", "adaptor", "all"],
        required=True,
        help="Contract type to transfer: 'liquidity_manager', 'adaptor', or 'all'",
    )
    args, forge_args = parser.parse_known_args()

    tokens_path: Path
    if args.tokens_file:
        tokens_path = Path(args.tokens_file).expanduser()
    elif args.positional_file:
        tokens_path = Path(args.positional_file).expanduser()
    else:
        tokens_path = DEFAULT_TOKENS_FILE

    if not tokens_path.is_absolute():
        tokens_path = Path.cwd() / tokens_path

    if forge_args and forge_args[0] == "--":
        forge_args = forge_args[1:]

    new_owner = os.environ.get("NEW_OWNER")
    if not new_owner:
        raise ConfigError("NEW_OWNER environment variable must be set")

    return tokens_path, new_owner, args.type, forge_args


SIGNER_FLAGS = {
    "--private-key",
    "--mnemonic",
    "--mnemonic-indexes",
    "--mnemonic-derivation-path",
    "--mnemonic-passphrase",
    "--ledger",
    "--trezor",
    "--keystore",
    "--keystore-password",
    "--keystore-account",
}


def has_signer_flag(forge_args: Sequence[str]) -> bool:
    for arg in forge_args:
        flag = arg.split("=", 1)[0]
        if flag in SIGNER_FLAGS:
            return True
    return False


def ensure_private_key(forge_args: Sequence[str]) -> None:
    if has_signer_flag(forge_args):
        return
    if not os.environ.get("PRIVATE_KEY"):
        raise ConfigError("PRIVATE_KEY environment variable must be set for forge broadcast")


def ensure_command_available(name: str) -> None:
    if shutil.which(name) is None:
        raise ConfigError(f"{name} is required but was not found in PATH")


def normalize_str(value: Any) -> str | None:
    if value is None:
        return None
    if isinstance(value, str):
        trimmed = value.strip()
        return trimmed or None
    return str(value)


def normalize_bool(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        return value.strip().lower() in {"1", "true", "yes", "y", "on"}
    if isinstance(value, (int, float)):
        return bool(value)
    return False


def expand_env_vars(value: str) -> str:
    """Expand ${VAR} patterns with environment variable values."""
    def replace(match: re.Match[str]) -> str:
        var_name = match.group(1)
        env_value = os.environ.get(var_name)
        if env_value is None:
            raise ConfigError(f"environment variable '{var_name}' is not set")
        return env_value
    return re.sub(r'\$\{(\w+)\}', replace, value)


def first_rpc_url(value: Any) -> str | None:
    if isinstance(value, list):
        url = normalize_str(value[0]) if value else None
    elif isinstance(value, str):
        url = normalize_str(value)
    else:
        url = None
    if url:
        url = expand_env_vars(url)
    return url


def parse_config(tokens_path: Path, contract_type: str) -> list[ContractConfig]:
    try:
        raw = json.loads(tokens_path.read_text())
    except FileNotFoundError as exc:
        raise ConfigError(f"tokens file not found at {tokens_path}") from exc
    except json.JSONDecodeError as exc:
        raise ConfigError(f"failed to parse JSON from {tokens_path}: {exc}") from exc

    tokens_raw = raw.get("tokens")
    if not tokens_raw:
        raise ConfigError(f"tokens array is empty in {tokens_path}")

    contracts: list[ContractConfig] = []
    for entry in tokens_raw:
        label = normalize_str(entry.get("label"))
        if not label:
            raise ConfigError("token entry missing label")

        chain_id = normalize_str(entry.get("chain_id") or entry.get("chainId"))
        if not chain_id or chain_id == "null":
            raise ConfigError(f"token '{label}' missing chain_id")

        rpc_url = first_rpc_url(entry.get("rpc_urls") or entry.get("rpcUrls"))
        if not rpc_url:
            raise ConfigError(f"token '{label}' must configure rpc_urls")

        legacy_raw = entry.get("legacy_tx")
        if legacy_raw is None:
            legacy_raw = entry.get("legacyTx")
        legacy_tx = normalize_bool(legacy_raw)

        # Add LiquidityManager
        if contract_type in ("liquidity_manager", "all"):
            lm_address = normalize_str(
                entry.get("liquidity_manager_address") or entry.get("liquidityManagerAddress")
            )
            if lm_address:
                contracts.append(
                    ContractConfig(
                        label=label,
                        contract_address=lm_address,
                        contract_type="liquidity_manager",
                        chain_id=chain_id,
                        rpc_url=rpc_url,
                        legacy_tx=legacy_tx,
                    )
                )

        # Add Adaptor
        if contract_type in ("adaptor", "all"):
            adaptor_address = normalize_str(
                entry.get("adaptor_address") or entry.get("adaptorAddress")
            )
            if adaptor_address:
                contracts.append(
                    ContractConfig(
                        label=label,
                        contract_address=adaptor_address,
                        contract_type="adaptor",
                        chain_id=chain_id,
                        rpc_url=rpc_url,
                        legacy_tx=legacy_tx,
                    )
                )

    return contracts


def run_forge(script: str, rpc_url: str, forge_args: Sequence[str], env_overrides: dict[str, str]) -> None:
    env = os.environ.copy()
    env.update(env_overrides)
    cmd = ["forge", "script", script, "--rpc-url", rpc_url]
    if not has_signer_flag(forge_args):
        ensure_private_key(forge_args)
    cmd.extend(forge_args)
    subprocess.run(cmd, cwd=SCRIPT_DIR, env=env, check=True)


def main() -> None:
    tokens_path, new_owner, contract_type, forge_args = parse_args()
    if not tokens_path.is_file():
        raise ConfigError(f"tokens file not found at {tokens_path}")

    ensure_command_available("forge")
    ensure_private_key(forge_args)

    contracts = parse_config(tokens_path, contract_type)
    if not contracts:
        print("No LiquidityManagers or Adaptors found. Nothing to do.")
        return

    base_forge_args = forge_args if forge_args else ["--broadcast"]

    print(f"Transferring ownership to {new_owner} on {len(contracts)} contract(s)")

    for contract in contracts:
        forge_args_for_chain = list(base_forge_args)
        if contract.legacy_tx:
            forge_args_for_chain.append("--legacy")

        if contract.contract_type == "liquidity_manager":
            script = TRANSFER_LM_OWNERSHIP_SCRIPT
            type_desc = "LiquidityManager"
        else:
            script = TRANSFER_ADAPTOR_OWNERSHIP_SCRIPT
            type_desc = "Adaptor"

        prefix = f"Transferring {type_desc} ownership for '{contract.label}' ({contract.contract_address})"
        print(f"{prefix} (legacy tx)" if contract.legacy_tx else prefix)

        run_forge(
            script=script,
            rpc_url=contract.rpc_url,
            forge_args=forge_args_for_chain,
            env_overrides={
                "CONTRACT_ADDRESS": contract.contract_address,
                "NEW_OWNER": new_owner,
            },
        )

    print("Ownership transfer scripts completed")


if __name__ == "__main__":
    try:
        main()
    except ConfigError as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)
    except subprocess.CalledProcessError as exc:
        cmd = " ".join(exc.cmd) if isinstance(exc.cmd, (list, tuple)) else str(exc.cmd)
        print(f"forge command failed with exit code {exc.returncode}: {cmd}", file=sys.stderr)
        sys.exit(exc.returncode)
