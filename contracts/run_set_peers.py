#!/usr/bin/env python3
"""Run SetPeers.s.sol helpers based on a tokens.json configuration."""
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
SET_PEERS_SCRIPT = "script/SetPeers.s.sol"


class ConfigError(RuntimeError):
    """Raised when the tokens file is missing required fields."""


@dataclass
class HubConfig:
    address: str
    chain_id: str
    eid: str
    rpc_url: str


@dataclass
class TokenConfig:
    label: str
    verifier_address: str
    token_address: str
    chain_id: str
    rpc_url: str
    eid: str
    legacy_tx: bool


def parse_args() -> tuple[Path, list[str]]:
    parser = argparse.ArgumentParser(
        usage="run_set_peers.py [--file PATH] [--] [forge flags...]",
        description=(
            "Reads a tokens.json-formatted file (requires per-entry EIDs) and runs "
            "SetPeers.s.sol (SetHubPeers once, SetVerifierPeers per token, SetTokenPeers per token) "
            "with environment variables derived from the configuration."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Examples:\n"
            "  ./run_set_peers.py\n"
            "  ./run_set_peers.py --file ../config/tokens.prod.json -- --broadcast -vv\n"
            "  # Defaults add '--broadcast' when no forge flags are provided\n"
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

    return tokens_path, forge_args


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


def parse_config(tokens_path: Path) -> tuple[HubConfig, list[TokenConfig]]:
    try:
        raw = json.loads(tokens_path.read_text())
    except FileNotFoundError as exc:
        raise ConfigError(f"tokens file not found at {tokens_path}") from exc
    except json.JSONDecodeError as exc:
        raise ConfigError(f"failed to parse JSON from {tokens_path}: {exc}") from exc

    hub_raw = raw.get("hub") or {}
    hub_address = normalize_str(hub_raw.get("hub_address") or hub_raw.get("hubAddress"))
    if not hub_address:
        raise ConfigError(f"hub_address missing from {tokens_path}")

    hub_chain_id = normalize_str(hub_raw.get("chain_id") or hub_raw.get("chainId"))
    if not hub_chain_id or hub_chain_id == "null":
        raise ConfigError(f"hub.chain_id missing from {tokens_path}")

    hub_rpc = first_rpc_url(hub_raw.get("rpc_urls") or hub_raw.get("rpcUrls"))
    if not hub_rpc:
        raise ConfigError(f"unable to resolve hub RPC endpoint from hub.rpc_urls in {tokens_path}")

    hub_eid = normalize_str(hub_raw.get("eid"))
    if not hub_eid or hub_eid == "null":
        raise ConfigError(f"hub.eid missing from {tokens_path}")

    tokens_raw = raw.get("tokens")
    if not tokens_raw:
        raise ConfigError(f"tokens array is empty in {tokens_path}")

    tokens: list[TokenConfig] = []
    for entry in tokens_raw:
        label = normalize_str(entry.get("label"))
        if not label:
            raise ConfigError("token entry missing label")

        verifier_address = normalize_str(entry.get("verifier_address") or entry.get("verifierAddress"))
        if not verifier_address:
            raise ConfigError(f"token '{label}' missing verifier_address")

        token_address = normalize_str(entry.get("token_address") or entry.get("tokenAddress"))
        if not token_address:
            raise ConfigError(f"token '{label}' missing token_address")

        chain_id = normalize_str(entry.get("chain_id") or entry.get("chainId"))
        if not chain_id or chain_id == "null":
            raise ConfigError(f"token '{label}' missing chain_id")

        rpc_url = first_rpc_url(entry.get("rpc_urls") or entry.get("rpcUrls"))
        if not rpc_url:
            raise ConfigError(f"token '{label}' must configure rpc_urls")

        eid = normalize_str(entry.get("eid"))
        if not eid or eid == "null":
            raise ConfigError(f"token '{label}' missing eid")

        legacy_raw = entry.get("legacy_tx")
        if legacy_raw is None:
            legacy_raw = entry.get("legacyTx")
        legacy_tx = normalize_bool(legacy_raw)

        tokens.append(
            TokenConfig(
                label=label,
                verifier_address=verifier_address,
                token_address=token_address,
                chain_id=chain_id,
                rpc_url=rpc_url,
                eid=eid,
                legacy_tx=legacy_tx,
            )
        )

    return HubConfig(address=hub_address, chain_id=hub_chain_id, eid=hub_eid, rpc_url=hub_rpc), tokens


def join_by_comma(values: Sequence[str]) -> str:
    return ",".join(values)


def run_forge(target: str, rpc_url: str, forge_args: Sequence[str], env_overrides: dict[str, str]) -> None:
    env = os.environ.copy()
    env.update(env_overrides)
    cmd = ["forge", "script", f"{SET_PEERS_SCRIPT}:{target}", "--rpc-url", rpc_url]
    if not has_signer_flag(forge_args):
        ensure_private_key(forge_args)
    cmd.extend(forge_args)
    subprocess.run(cmd, cwd=SCRIPT_DIR, env=env, check=True)


def main() -> None:
    tokens_path, forge_args = parse_args()
    if not tokens_path.is_file():
        raise ConfigError(f"tokens file not found at {tokens_path}")

    ensure_command_available("forge")
    ensure_private_key(forge_args)

    hub, tokens = parse_config(tokens_path)
    base_forge_args = forge_args if forge_args else ["--broadcast"]

    verifier_addresses = [token.verifier_address for token in tokens]
    token_addresses = [token.token_address for token in tokens]
    token_chain_ids = [token.chain_id for token in tokens]
    verifier_eids = [token.eid for token in tokens]

    print(f"Running SetHubPeers against {hub.rpc_url} for {len(tokens)} token(s)")
    run_forge(
        target="SetHubPeers",
        rpc_url=hub.rpc_url,
        forge_args=base_forge_args,
        env_overrides={
            "HUB_ADDRESS": hub.address,
            "VERIFIER_ADDRESSES": join_by_comma(verifier_addresses),
            "VERIFIER_EIDS": join_by_comma(verifier_eids),
            "TOKEN_ADDRESSES": join_by_comma(token_addresses),
            "TOKEN_CHAIN_IDS": join_by_comma(token_chain_ids),
        },
    )

    for token in tokens:
        forge_args_for_token = list(base_forge_args)
        if token.legacy_tx:
            forge_args_for_token.append("--legacy")

        prefix = f"Running SetVerifierPeers for '{token.label}' via {token.rpc_url}"
        print(f"{prefix} (legacy tx)" if token.legacy_tx else prefix)
        run_forge(
            target="SetVerifierPeers",
            rpc_url=token.rpc_url,
            forge_args=forge_args_for_token,
            env_overrides={
                "HUB_ADDRESS": hub.address,
                "HUB_EID": hub.eid,
                "VERIFIER_ADDRESS": token.verifier_address,
            },
        )

    if len(tokens) > 1:
        peer_count = len(tokens) - 1
        print(f"Running SetTokenPeers for {len(tokens)} chain(s); each token will set {peer_count} peer(s)")
        for idx, token in enumerate(tokens):
            peer_eids = [peer.eid for peer_index, peer in enumerate(tokens) if peer_index != idx]
            peer_addrs = [peer.token_address for peer_index, peer in enumerate(tokens) if peer_index != idx]

            forge_args_for_token = list(base_forge_args)
            if token.legacy_tx:
                forge_args_for_token.append("--legacy")

            prefix = f"Running SetTokenPeers for '{token.label}' via {token.rpc_url}"
            print(f"{prefix} (legacy tx)" if token.legacy_tx else prefix)
            run_forge(
                target="SetTokenPeers",
                rpc_url=token.rpc_url,
                forge_args=forge_args_for_token,
                env_overrides={
                    "TOKEN_ADDRESS": token.token_address,
                    "PEER_ADDRESSES": join_by_comma(peer_addrs),
                    "PEER_EIDS": join_by_comma(peer_eids),
                },
            )
    else:
        print("Skipping SetTokenPeers: only one token entry present")

    print("SetPeers scripts completed")


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
