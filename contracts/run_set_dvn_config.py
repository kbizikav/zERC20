#!/usr/bin/env python3
"""Run SetDvnConfig.s.sol based on tokens.json and per-chain DVN config."""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Sequence


SCRIPT_DIR = Path(__file__).resolve().parent
ROOT_DIR = SCRIPT_DIR
DEFAULT_CONFIG_FILE = (ROOT_DIR / "config" / "dvn-config.json").resolve()
SET_DVN_SCRIPT = "script/SetDvnConfig.s.sol"


class ConfigError(RuntimeError):
    """Raised when the config file is missing required fields."""


@dataclass
class HubConfig:
    address: str
    eid: str
    rpc_url: str
    legacy_tx: bool


@dataclass
class TokenConfig:
    label: str
    verifier_address: str
    token_address: str
    eid: str
    rpc_url: str
    legacy_tx: bool


@dataclass
class DvnPolicy:
    confirmations: int
    required_dvns: list[str]
    optional_dvns: list[str]
    optional_threshold: int


@dataclass
class ChainPolicies:
    label: str
    verifier_hub: DvnPolicy
    token: DvnPolicy


def parse_args() -> tuple[Path, list[str]]:
    parser = argparse.ArgumentParser(
        usage="run_set_dvn_config.py [--file PATH] [--] [forge flags...]",
        description=(
            "Reads a dvn-config.json file (per-chain policies) and runs SetDvnConfig.s.sol "
            "using data from tokens.json."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Examples:\n"
            "  ./run_set_dvn_config.py\n"
            "  ./run_set_dvn_config.py --file ../config/dvn-config.prod.json -- --broadcast -vv\n"
            "  # Defaults add '--broadcast' when no forge flags are provided\n"
        ),
    )
    parser.add_argument("--file", dest="config_file", help="Path to dvn-config.json (defaults to ../config/dvn-config.json)")
    parser.add_argument("positional_file", nargs="?", help="Optional dvn-config.json path when not using --file")
    args, forge_args = parser.parse_known_args()

    config_path: Path
    if args.config_file:
        config_path = Path(args.config_file).expanduser()
    elif args.positional_file:
        config_path = Path(args.positional_file).expanduser()
    else:
        config_path = DEFAULT_CONFIG_FILE

    if not config_path.is_absolute():
        config_path = Path.cwd() / config_path

    if forge_args and forge_args[0] == "--":
        forge_args = forge_args[1:]

    return config_path, forge_args


def ensure_private_key() -> None:
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


def normalize_int(value: Any, name: str) -> int:
    if value is None:
        raise ConfigError(f"{name} is required")
    if isinstance(value, bool):
        raise ConfigError(f"{name} must be a number")
    try:
        return int(value)
    except (TypeError, ValueError) as exc:
        raise ConfigError(f"{name} must be a number") from exc


def normalize_list(value: Any, name: str) -> list[str]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise ConfigError(f"{name} must be an array")
    items: list[str] = []
    for entry in value:
        item = normalize_str(entry)
        if item is None:
            raise ConfigError(f"{name} contains an empty entry")
        items.append(item)
    return items


def first_rpc_url(value: Any) -> str | None:
    if isinstance(value, list):
        return normalize_str(value[0]) if value else None
    if isinstance(value, str):
        return normalize_str(value)
    return None


def parse_policy(entry: Any, label: str) -> DvnPolicy:
    if not isinstance(entry, dict):
        raise ConfigError(f"{label} must be an object")

    confirmations = normalize_int(entry.get("confirmations"), f"{label} confirmations")
    required_dvns = normalize_list(entry.get("required_dvns") or entry.get("requiredDvns"), f"{label} required_dvns")
    optional_dvns = normalize_list(entry.get("optional_dvns") or entry.get("optionalDvns"), f"{label} optional_dvns")
    optional_threshold_raw = entry.get("optional_threshold")
    if optional_threshold_raw is None:
        optional_threshold_raw = entry.get("optionalThreshold")
    optional_threshold = (
        normalize_int(optional_threshold_raw, f"{label} optional_threshold")
        if optional_threshold_raw is not None
        else 0
    )

    if confirmations < 0:
        raise ConfigError(f"{label} confirmations must be >= 0")
    if optional_threshold < 0:
        raise ConfigError(f"{label} optional_threshold must be >= 0")
    if not required_dvns and not optional_dvns:
        raise ConfigError(f"{label} must configure required_dvns or optional_dvns")
    if optional_dvns and optional_threshold == 0:
        raise ConfigError(f"{label} optional_threshold is required when optional_dvns is set")
    if not optional_dvns and optional_threshold != 0:
        raise ConfigError(f"{label} optional_threshold must be 0 when optional_dvns is empty")
    if optional_threshold > len(optional_dvns):
        raise ConfigError(f"{label} optional_threshold exceeds optional_dvns length")

    return DvnPolicy(
        confirmations=confirmations,
        required_dvns=required_dvns,
        optional_dvns=optional_dvns,
        optional_threshold=optional_threshold,
    )


def parse_dvn_config(config_path: Path) -> tuple[Path, dict[str, ChainPolicies], DvnPolicy | None]:
    try:
        raw = json.loads(config_path.read_text())
    except FileNotFoundError as exc:
        raise ConfigError(f"config file not found at {config_path}") from exc
    except json.JSONDecodeError as exc:
        raise ConfigError(f"failed to parse JSON from {config_path}: {exc}") from exc

    tokens_file = normalize_str(raw.get("tokens_file") or raw.get("tokensFile"))
    if not tokens_file:
        raise ConfigError("tokens_file is required")

    tokens_path = Path(tokens_file).expanduser()
    if not tokens_path.is_absolute():
        tokens_path = config_path.parent / tokens_path

    chains_raw = raw.get("chains")
    if not isinstance(chains_raw, dict) or not chains_raw:
        raise ConfigError("chains must be a non-empty object")

    chains: dict[str, ChainPolicies] = {}
    for name, entry in chains_raw.items():
        label = normalize_str(name)
        if not label:
            raise ConfigError("chain name missing in chains")
        if not isinstance(entry, dict):
            raise ConfigError(f"chain '{label}' must be an object")

        verifier_entry = entry.get("verifier_hub")
        if verifier_entry is None:
            verifier_entry = entry.get("verifierHub")
        token_entry = entry.get("token")

        if verifier_entry is None:
            raise ConfigError(f"chain '{label}' missing verifier_hub config")
        if token_entry is None:
            raise ConfigError(f"chain '{label}' missing token config")

        verifier_policy = parse_policy(verifier_entry, f"chain '{label}' verifier_hub")
        token_policy = parse_policy(token_entry, f"chain '{label}' token")

        chains[label] = ChainPolicies(label=label, verifier_hub=verifier_policy, token=token_policy)

    hub_policy = None
    hub_raw = raw.get("hub")
    if hub_raw is not None:
        if not isinstance(hub_raw, dict):
            raise ConfigError("hub must be an object")
        hub_entry = hub_raw.get("verifier_hub")
        if hub_entry is None:
            hub_entry = hub_raw.get("verifierHub")
        if hub_entry is None:
            raise ConfigError("hub missing verifier_hub config")
        hub_policy = parse_policy(hub_entry, "hub verifier_hub")

    return tokens_path, chains, hub_policy


def parse_tokens(tokens_path: Path) -> tuple[HubConfig, list[TokenConfig]]:
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

    hub_eid = normalize_str(hub_raw.get("eid"))
    if not hub_eid or hub_eid == "null":
        raise ConfigError(f"hub.eid missing from {tokens_path}")

    hub_rpc = first_rpc_url(hub_raw.get("rpc_urls") or hub_raw.get("rpcUrls") or hub_raw.get("rpc_url") or hub_raw.get("rpcUrl"))
    if not hub_rpc:
        raise ConfigError(f"unable to resolve hub RPC endpoint from hub.rpc_urls in {tokens_path}")

    hub_legacy_raw = hub_raw.get("legacy_tx")
    if hub_legacy_raw is None:
        hub_legacy_raw = hub_raw.get("legacyTx")
    hub_legacy = normalize_bool(hub_legacy_raw)

    tokens_raw = raw.get("tokens")
    if not isinstance(tokens_raw, list) or not tokens_raw:
        raise ConfigError(f"tokens array is empty in {tokens_path}")

    tokens: list[TokenConfig] = []
    labels_seen: set[str] = set()
    for entry in tokens_raw:
        if not isinstance(entry, dict):
            raise ConfigError("token entry must be an object")

        label = normalize_str(entry.get("label"))
        if not label:
            raise ConfigError("token entry missing label")
        if label in labels_seen:
            raise ConfigError(f"duplicate token label '{label}' in {tokens_path}")
        labels_seen.add(label)

        verifier_address = normalize_str(entry.get("verifier_address") or entry.get("verifierAddress"))
        if not verifier_address:
            raise ConfigError(f"token '{label}' missing verifier_address")

        token_address = normalize_str(entry.get("token_address") or entry.get("tokenAddress"))
        if not token_address:
            raise ConfigError(f"token '{label}' missing token_address")

        eid = normalize_str(entry.get("eid"))
        if not eid or eid == "null":
            raise ConfigError(f"token '{label}' missing eid")

        rpc_url = first_rpc_url(entry.get("rpc_urls") or entry.get("rpcUrls") or entry.get("rpc_url") or entry.get("rpcUrl"))
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
                token_address=token_address,
                eid=eid,
                rpc_url=rpc_url,
                legacy_tx=legacy_tx,
            )
        )

    return HubConfig(address=hub_address, eid=hub_eid, rpc_url=hub_rpc, legacy_tx=hub_legacy), tokens


def join_by_comma(values: Sequence[str]) -> str:
    return ",".join(values)


def run_forge(target: str, rpc_url: str, forge_args: Sequence[str], env_overrides: dict[str, str]) -> None:
    env = os.environ.copy()
    env.update(env_overrides)
    private_key = os.environ.get("PRIVATE_KEY", "")
    cmd = ["forge", "script", f"{SET_DVN_SCRIPT}:{target}", "--rpc-url", rpc_url, "--private-key", private_key, *forge_args]
    subprocess.run(cmd, cwd=SCRIPT_DIR, env=env, check=True)


def policy_env(policy: DvnPolicy) -> dict[str, str]:
    env: dict[str, str] = {"CONFIRMATIONS": str(policy.confirmations)}
    if policy.required_dvns:
        env["REQUIRED_DVN_NAMES"] = join_by_comma(policy.required_dvns)
    if policy.optional_dvns:
        env["OPTIONAL_DVN_NAMES"] = join_by_comma(policy.optional_dvns)
    if policy.optional_threshold:
        env["OPTIONAL_DVN_THRESHOLD"] = str(policy.optional_threshold)
    return env


def apply_config(
    *,
    label: str,
    rpc_url: str,
    forge_args: Sequence[str],
    oapp: str,
    remote_eid: str,
    policy: DvnPolicy,
    target_lib: str | None = None,
) -> None:
    print(f"Setting DVN config for {label} via {rpc_url}")
    env_overrides = {
        "OAPP_ADDRESS": oapp,
        "REMOTE_EID": remote_eid,
        **policy_env(policy),
    }
    if target_lib:
        env_overrides["TARGET_LIB"] = target_lib
    run_forge(
        target="SetDvnConfig",
        rpc_url=rpc_url,
        forge_args=forge_args,
        env_overrides=env_overrides,
    )


def main() -> None:
    config_path, forge_args = parse_args()
    if not config_path.is_file():
        raise ConfigError(f"config file not found at {config_path}")

    ensure_command_available("forge")
    ensure_private_key()

    tokens_path, chain_policies, hub_policy = parse_dvn_config(config_path)
    hub, tokens = parse_tokens(tokens_path)

    token_labels = {token.label for token in tokens}
    policy_labels = set(chain_policies.keys())
    missing = token_labels - policy_labels
    extra = policy_labels - token_labels
    if missing:
        missing_list = ", ".join(sorted(missing))
        raise ConfigError(f"dvn-config missing chains: {missing_list}")
    if extra:
        extra_list = ", ".join(sorted(extra))
        raise ConfigError(f"dvn-config has unknown chains: {extra_list}")

    base_forge_args = forge_args if forge_args else ["--broadcast"]

    print(f"Running verifier<->hub config for {len(tokens)} chain(s)")
    if hub_policy is None:
        print("Warning: hub.verifier_hub not configured; using per-chain verifier_hub policy for hub -> verifier")
    for token in tokens:
        policy = chain_policies[token.label].verifier_hub
        hub_direction_policy = hub_policy or policy

        hub_args = list(base_forge_args)
        if hub.legacy_tx:
            hub_args.append("--legacy")

        verifier_args = list(base_forge_args)
        if token.legacy_tx:
            verifier_args.append("--legacy")

        apply_config(
            label=f"hub -> verifier ({token.label}) send",
            rpc_url=hub.rpc_url,
            forge_args=hub_args,
            oapp=hub.address,
            remote_eid=token.eid,
            policy=hub_direction_policy,
            target_lib="send",
        )
        apply_config(
            label=f"hub -> verifier ({token.label}) receive",
            rpc_url=token.rpc_url,
            forge_args=verifier_args,
            oapp=token.verifier_address,
            remote_eid=hub.eid,
            policy=hub_direction_policy,
            target_lib="receive",
        )

        apply_config(
            label=f"verifier -> hub ({token.label}) send",
            rpc_url=token.rpc_url,
            forge_args=verifier_args,
            oapp=token.verifier_address,
            remote_eid=hub.eid,
            policy=policy,
            target_lib="send",
        )
        apply_config(
            label=f"verifier -> hub ({token.label}) receive",
            rpc_url=hub.rpc_url,
            forge_args=hub_args,
            oapp=hub.address,
            remote_eid=token.eid,
            policy=policy,
            target_lib="receive",
        )

    if len(tokens) > 1:
        total_routes = len(tokens) * (len(tokens) - 1)
        print(f"Running token<->token config for {total_routes} route(s)")
        for src in tokens:
            policy = chain_policies[src.label].token
            src_args = list(base_forge_args)
            if src.legacy_tx:
                src_args.append("--legacy")
            for dst in tokens:
                if dst.label == src.label:
                    continue
                apply_config(
                    label=f"token {src.label} -> {dst.label} send",
                    rpc_url=src.rpc_url,
                    forge_args=src_args,
                    oapp=src.token_address,
                    remote_eid=dst.eid,
                    policy=policy,
                    target_lib="send",
                )
                dst_args = list(base_forge_args)
                if dst.legacy_tx:
                    dst_args.append("--legacy")
                apply_config(
                    label=f"token {src.label} -> {dst.label} receive",
                    rpc_url=dst.rpc_url,
                    forge_args=dst_args,
                    oapp=dst.token_address,
                    remote_eid=src.eid,
                    policy=policy,
                    target_lib="receive",
                )
    else:
        print("Skipping token<->token config: only one token chain")

    print("DVN config scripts completed")


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
