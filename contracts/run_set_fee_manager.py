#!/usr/bin/env python3
"""Run SetFeeManager.s.sol for all chains with liquidity_manager_address in tokens.json."""
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
SET_FEE_MANAGER_SCRIPT = "script/SetFeeManager.s.sol"


class ConfigError(RuntimeError):
    """Raised when the configuration is invalid or missing required fields."""


@dataclass
class ChainConfig:
    label: str
    liquidity_manager_address: str
    chain_id: str
    rpc_url: str
    legacy_tx: bool


def parse_args() -> tuple[Path, str, list[str]]:
    parser = argparse.ArgumentParser(
        usage="run_set_fee_manager.py [--file PATH] [--] [forge flags...]",
        description=(
            "Reads a tokens.json-formatted file and runs SetFeeManager.s.sol "
            "for each chain that has a liquidity_manager_address configured."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Examples:\n"
            "  FEE_MANAGER=0x123... ./run_set_fee_manager.py\n"
            "  FEE_MANAGER=0x123... ./run_set_fee_manager.py --file ../config/tokens.prod.json -- --broadcast -vv\n"
        ),
    )
    parser.add_argument("--file", dest="tokens_file", help="Path to tokens.json (defaults to ../config/tokens.json)")
    parser.add_argument("positional_file", nargs="?", help="Optional tokens.json path when not using --file")
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

    fee_manager = os.environ.get("FEE_MANAGER")
    if not fee_manager:
        raise ConfigError("FEE_MANAGER environment variable must be set")

    return tokens_path, fee_manager, forge_args


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


def parse_config(tokens_path: Path) -> list[ChainConfig]:
    try:
        raw = json.loads(tokens_path.read_text())
    except FileNotFoundError as exc:
        raise ConfigError(f"tokens file not found at {tokens_path}") from exc
    except json.JSONDecodeError as exc:
        raise ConfigError(f"failed to parse JSON from {tokens_path}: {exc}") from exc

    tokens_raw = raw.get("tokens")
    if not tokens_raw:
        raise ConfigError(f"tokens array is empty in {tokens_path}")

    chains: list[ChainConfig] = []
    for entry in tokens_raw:
        label = normalize_str(entry.get("label"))
        if not label:
            raise ConfigError("token entry missing label")

        liquidity_manager_address = normalize_str(
            entry.get("liquidity_manager_address") or entry.get("liquidityManagerAddress")
        )
        if not liquidity_manager_address:
            print(f"Skipping '{label}': no liquidity_manager_address configured")
            continue

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

        chains.append(
            ChainConfig(
                label=label,
                liquidity_manager_address=liquidity_manager_address,
                chain_id=chain_id,
                rpc_url=rpc_url,
                legacy_tx=legacy_tx,
            )
        )

    return chains


def run_forge(rpc_url: str, forge_args: Sequence[str], env_overrides: dict[str, str]) -> None:
    env = os.environ.copy()
    env.update(env_overrides)
    cmd = ["forge", "script", SET_FEE_MANAGER_SCRIPT, "--rpc-url", rpc_url]
    if not has_signer_flag(forge_args):
        ensure_private_key(forge_args)
    cmd.extend(forge_args)
    subprocess.run(cmd, cwd=SCRIPT_DIR, env=env, check=True)


def main() -> None:
    tokens_path, fee_manager, forge_args = parse_args()
    if not tokens_path.is_file():
        raise ConfigError(f"tokens file not found at {tokens_path}")

    ensure_command_available("forge")
    ensure_private_key(forge_args)

    chains = parse_config(tokens_path)
    if not chains:
        print("No chains with liquidity_manager_address found. Nothing to do.")
        return

    base_forge_args = forge_args if forge_args else ["--broadcast"]

    print(f"Setting FEE_MANAGER_ROLE to {fee_manager} on {len(chains)} chain(s)")

    for chain in chains:
        forge_args_for_chain = list(base_forge_args)
        if chain.legacy_tx:
            forge_args_for_chain.append("--legacy")

        prefix = f"Running SetFeeManager for '{chain.label}' via {chain.rpc_url}"
        print(f"{prefix} (legacy tx)" if chain.legacy_tx else prefix)

        run_forge(
            rpc_url=chain.rpc_url,
            forge_args=forge_args_for_chain,
            env_overrides={
                "LIQUIDITY_MANAGER": chain.liquidity_manager_address,
                "FEE_MANAGER": fee_manager,
            },
        )

    print("SetFeeManager scripts completed")


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
