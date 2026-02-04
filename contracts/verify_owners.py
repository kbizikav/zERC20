#!/usr/bin/env python3
"""Verify that all OApp owners in tokens.json match the expected NEW_OWNER."""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

try:
    from web3 import Web3
except ImportError:
    print("error: web3 is required. Install with: pip install web3", file=sys.stderr)
    sys.exit(1)


SCRIPT_DIR = Path(__file__).resolve().parent
ROOT_DIR = SCRIPT_DIR.parent
DEFAULT_TOKENS_FILE = (ROOT_DIR / "config" / "tokens.json").resolve()

# Ownable.owner() function signature
OWNER_SELECTOR = "0x8da5cb5b"


class ConfigError(RuntimeError):
    """Raised when the configuration is invalid or missing required fields."""


@dataclass
class OAppConfig:
    label: str
    oapp_address: str
    oapp_type: str  # "token" or "verifier"
    chain_id: str
    rpc_url: str


def parse_args() -> tuple[Path, str]:
    parser = argparse.ArgumentParser(
        usage="verify_oapp_owners.py [--file PATH]",
        description="Verify that all OApp owners in tokens.json match NEW_OWNER.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Examples:\n"
            "  NEW_OWNER=0x123... ./verify_oapp_owners.py\n"
            "  NEW_OWNER=0x123... ./verify_oapp_owners.py --file ../config/tokens.prod.json\n"
        ),
    )
    parser.add_argument("--file", dest="tokens_file", help="Path to tokens.json (defaults to ../config/tokens.json)")
    parser.add_argument("positional_file", nargs="?", help="Optional tokens.json path when not using --file")
    args = parser.parse_args()

    tokens_path: Path
    if args.tokens_file:
        tokens_path = Path(args.tokens_file).expanduser()
    elif args.positional_file:
        tokens_path = Path(args.positional_file).expanduser()
    else:
        tokens_path = DEFAULT_TOKENS_FILE

    if not tokens_path.is_absolute():
        tokens_path = Path.cwd() / tokens_path

    new_owner = os.environ.get("NEW_OWNER")
    if not new_owner:
        raise ConfigError("NEW_OWNER environment variable must be set")

    return tokens_path, new_owner


def normalize_str(value: Any) -> str | None:
    if value is None:
        return None
    if isinstance(value, str):
        trimmed = value.strip()
        return trimmed or None
    return str(value)


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


def parse_config(tokens_path: Path) -> list[OAppConfig]:
    try:
        raw = json.loads(tokens_path.read_text())
    except FileNotFoundError as exc:
        raise ConfigError(f"tokens file not found at {tokens_path}") from exc
    except json.JSONDecodeError as exc:
        raise ConfigError(f"failed to parse JSON from {tokens_path}: {exc}") from exc

    tokens_raw = raw.get("tokens")
    if not tokens_raw:
        raise ConfigError(f"tokens array is empty in {tokens_path}")

    oapps: list[OAppConfig] = []
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

        # Add token
        token_address = normalize_str(
            entry.get("token_address") or entry.get("tokenAddress")
        )
        if token_address:
            oapps.append(
                OAppConfig(
                    label=label,
                    oapp_address=token_address,
                    oapp_type="token",
                    chain_id=chain_id,
                    rpc_url=rpc_url,
                )
            )

        # Add verifier
        verifier_address = normalize_str(
            entry.get("verifier_address") or entry.get("verifierAddress")
        )
        if verifier_address:
            oapps.append(
                OAppConfig(
                    label=label,
                    oapp_address=verifier_address,
                    oapp_type="verifier",
                    chain_id=chain_id,
                    rpc_url=rpc_url,
                )
            )

        # Add liquidity manager
        liquidity_address = normalize_str(
            entry.get("liquidity_manager_address") or entry.get("liquidityManagerAddress")
        )
        if liquidity_address:
            oapps.append(
                OAppConfig(
                    label=label,
                    oapp_address=liquidity_address,
                    oapp_type="liquidity",
                    chain_id=chain_id,
                    rpc_url=rpc_url,
                )
            )

        # Add adaptor
        adaptor_address = normalize_str(
            entry.get("adaptor_address") or entry.get("adaptorAddress")
        )
        if adaptor_address:
            oapps.append(
                OAppConfig(
                    label=label,
                    oapp_address=adaptor_address,
                    oapp_type="adaptor",
                    chain_id=chain_id,
                    rpc_url=rpc_url,
                )
            )

    return oapps


def get_owner(w3: Web3, address: str) -> str | None:
    """Call owner() on the contract and return the owner address."""
    try:
        result = w3.eth.call({"to": Web3.to_checksum_address(address), "data": OWNER_SELECTOR})
        if len(result) >= 32:
            return Web3.to_checksum_address("0x" + result[-20:].hex())
        return None
    except Exception as e:
        print(f"  Error calling owner(): {e}")
        return None


def main() -> None:
    tokens_path, new_owner = parse_args()
    if not tokens_path.is_file():
        raise ConfigError(f"tokens file not found at {tokens_path}")

    oapps = parse_config(tokens_path)
    if not oapps:
        print("No tokens or verifiers found. Nothing to verify.")
        return

    new_owner_checksum = Web3.to_checksum_address(new_owner)
    print(f"Verifying {len(oapps)} OApp(s) have owner: {new_owner_checksum}\n")

    mismatches: list[tuple[OAppConfig, str | None]] = []
    successes = 0

    # Cache Web3 instances by RPC URL
    w3_cache: dict[str, Web3] = {}

    for oapp in oapps:
        if oapp.rpc_url not in w3_cache:
            w3_cache[oapp.rpc_url] = Web3(Web3.HTTPProvider(oapp.rpc_url))

        w3 = w3_cache[oapp.rpc_url]
        current_owner = get_owner(w3, oapp.oapp_address)

        status = "✓" if current_owner == new_owner_checksum else "✗"
        print(f"[{status}] {oapp.label} {oapp.oapp_type} ({oapp.oapp_address})")
        print(f"    Current owner: {current_owner or 'unknown'}")

        if current_owner == new_owner_checksum:
            successes += 1
        else:
            mismatches.append((oapp, current_owner))

    print(f"\n{'='*60}")
    print(f"Results: {successes}/{len(oapps)} OApps have correct owner")

    if mismatches:
        print(f"\nMismatches ({len(mismatches)}):")
        for oapp, current in mismatches:
            print(f"  - {oapp.label} {oapp.oapp_type}: {current or 'unknown'} (expected {new_owner_checksum})")
        sys.exit(1)
    else:
        print("\nAll OApps have the expected owner!")


if __name__ == "__main__":
    try:
        main()
    except ConfigError as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)
