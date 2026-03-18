#!/usr/bin/env python3
"""Upgrade Verifier proxies to a pre-deployed implementation using a tokens.json configuration."""
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
UPGRADE_SCRIPT = "script/upgrade/UpgradeVerifierToImpl.s.sol:UpgradeVerifierToImpl"


class ConfigError(RuntimeError):
    """Raised when the tokens file is missing required fields."""


@dataclass
class TokenConfig:
    label: str
    verifier_address: str
    chain_id: str
    rpc_url: str
    legacy_tx: bool


def parse_args() -> tuple[Path, str, list[str]]:
    parser = argparse.ArgumentParser(
        usage="run_upgrade_verifier.py --impl ADDRESS [--file PATH] [--] [forge flags...]",
        description=(
            "Reads a tokens.json-formatted file and upgrades every Verifier proxy "
            "to the given pre-deployed implementation, calling initializeV2 in the same transaction."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Examples:\n"
            "  ./run_upgrade_verifier.py --impl 0x1234...abcd\n"
            "  ./run_upgrade_verifier.py --impl 0x1234...abcd --file ../config/deployed/testnet/tokens.zusdc.testnet.json -- --broadcast -vv\n"
            "  ./run_upgrade_verifier.py --impl 0x1234...abcd --chain sepolia  # upgrade only sepolia\n"
        ),
    )
    parser.add_argument("--impl", required=True, help="Address of the already-deployed Verifier implementation")
    parser.add_argument("--file", dest="tokens_file", help="Path to tokens.json (defaults to ../config/tokens.json)")
    parser.add_argument("--chain", dest="chain_filter", help="Only upgrade the verifier on this chain (matched against label)")
    args, forge_args = parser.parse_known_args()

    tokens_path: Path
    if args.tokens_file:
        tokens_path = Path(args.tokens_file).expanduser()
    else:
        tokens_path = DEFAULT_TOKENS_FILE

    if not tokens_path.is_absolute():
        tokens_path = Path.cwd() / tokens_path

    if forge_args and forge_args[0] == "--":
        forge_args = forge_args[1:]

    # Stash chain filter in env so main() can access it without threading through
    if args.chain_filter:
        os.environ["_UPGRADE_CHAIN_FILTER"] = args.chain_filter

    return tokens_path, args.impl, forge_args


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


def parse_config(tokens_path: Path) -> list[TokenConfig]:
    try:
        raw = json.loads(tokens_path.read_text())
    except FileNotFoundError as exc:
        raise ConfigError(f"tokens file not found at {tokens_path}") from exc
    except json.JSONDecodeError as exc:
        raise ConfigError(f"failed to parse JSON from {tokens_path}: {exc}") from exc

    tokens_raw = raw.get("tokens")
    if not tokens_raw:
        raise ConfigError(f"tokens array is empty in {tokens_path}")

    chain_filter = os.environ.get("_UPGRADE_CHAIN_FILTER")

    tokens: list[TokenConfig] = []
    for entry in tokens_raw:
        label = normalize_str(entry.get("label"))
        if not label:
            raise ConfigError("token entry missing label")

        if chain_filter and label != chain_filter:
            continue

        verifier_address = normalize_str(entry.get("verifier_address") or entry.get("verifierAddress"))
        if not verifier_address:
            raise ConfigError(f"token '{label}' missing verifier_address")

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

        tokens.append(
            TokenConfig(
                label=label,
                verifier_address=verifier_address,
                chain_id=chain_id,
                rpc_url=rpc_url,
                legacy_tx=legacy_tx,
            )
        )

    if chain_filter and not tokens:
        raise ConfigError(f"no token entry matched chain filter '{chain_filter}'")

    return tokens


def run_forge(rpc_url: str, forge_args: Sequence[str], env_overrides: dict[str, str]) -> None:
    env = os.environ.copy()
    env.update(env_overrides)
    cmd = ["forge", "script", UPGRADE_SCRIPT, "--rpc-url", rpc_url]
    if not has_signer_flag(forge_args):
        ensure_private_key(forge_args)
    cmd.extend(forge_args)
    subprocess.run(cmd, cwd=SCRIPT_DIR, env=env, check=True)


def main() -> None:
    tokens_path, new_impl, forge_args = parse_args()
    if not tokens_path.is_file():
        raise ConfigError(f"tokens file not found at {tokens_path}")

    ensure_command_available("forge")
    ensure_private_key(forge_args)

    tokens = parse_config(tokens_path)
    base_forge_args = forge_args if forge_args else ["--broadcast"]

    print(f"Upgrading {len(tokens)} Verifier(s) to impl {new_impl}")
    print()

    for token in tokens:
        forge_args_for_token = list(base_forge_args)
        if token.legacy_tx:
            forge_args_for_token.append("--legacy")

        suffix = " (legacy tx)" if token.legacy_tx else ""
        print(f"[{token.label}] Upgrading verifier {token.verifier_address}{suffix}")
        print(f"  RPC: {token.rpc_url}")

        run_forge(
            rpc_url=token.rpc_url,
            forge_args=forge_args_for_token,
            env_overrides={
                "VERIFIER_PROXY": token.verifier_address,
                "NEW_IMPL": new_impl,
            },
        )
        print(f"[{token.label}] Done")
        print()

    print(f"All {len(tokens)} Verifier upgrade(s) completed")


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
