#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: run-set-peers.sh [--file PATH] [--] [forge flags...]

Reads a tokens.json-formatted file (requires per-entry EIDs) and runs
SetPeers.s.sol (SetHubPeers once, SetVerifierPeers per token, SetTokenPeers per token)
with environment variables derived from the configuration.

Options:
  --file PATH          Path to tokens.json (defaults to ../config/tokens.json)
  --help            Show this help message and exit
  --                Stop option parsing; following args are passed to forge script

Environment:
  PRIVATE_KEY       Required. Used by forge when broadcasting transactions.

Examples:
  ./run-set-peers.sh
  ./run-set-peers.sh --file ../config/tokens.prod.json -- --broadcast -vv
  # Defaults add '--broadcast' when no forge flags are provided
EOF
}

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT_DIR=$(cd "$SCRIPT_DIR/.." && pwd)
TOKENS_FILE="$ROOT_DIR/config/tokens.json"
FORGE_ARGS=()

while (($#)); do
  case "$1" in
    --file)
      if (($# == 1)); then
        echo "error: --file expects a path" >&2
        exit 1
      fi
      TOKENS_FILE="$2"
      shift 2
      ;;
    --help)
      usage
      exit 0
      ;;
    --)
      shift
      FORGE_ARGS=("$@")
      break
      ;;
    -*)
      echo "error: unknown option '$1'" >&2
      usage >&2
      exit 1
      ;;
    *)
      TOKENS_FILE="$1"
      shift
      ;;
  esac
done

if [[ ! -f "$TOKENS_FILE" ]]; then
  echo "error: tokens file not found at $TOKENS_FILE" >&2
  exit 1
fi

if [[ ${#FORGE_ARGS[@]} -eq 0 ]]; then
  FORGE_ARGS=(--broadcast)
fi

for cmd in jq forge; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: $cmd is required but was not found in PATH" >&2
    exit 1
  fi
done

if [[ -z "${PRIVATE_KEY:-}" ]]; then
  echo "error: PRIVATE_KEY environment variable must be set for forge broadcast" >&2
  exit 1
fi

HUB_ADDRESS=$(jq -r '(.hub // empty) | (.hub_address // .hubAddress // empty)' "$TOKENS_FILE")
if [[ -z "$HUB_ADDRESS" ]]; then
  echo "error: hub_address missing from $TOKENS_FILE" >&2
  exit 1
fi

HUB_CHAIN_ID=$(jq -r '(.hub // empty) | (.chain_id // .chainId // empty)' "$TOKENS_FILE")
if [[ -z "$HUB_CHAIN_ID" || "$HUB_CHAIN_ID" == "null" ]]; then
  echo "error: hub.chain_id missing from $TOKENS_FILE" >&2
  exit 1
fi

HUB_RPC=$(jq -r '(.hub // empty) | (.rpc_urls // .rpcUrls // empty)
  | if type == "array" then (if length > 0 then .[0] else empty end)
    elif type == "string" then .
    else empty
    end' "$TOKENS_FILE")
if [[ -z "$HUB_RPC" ]]; then
  echo "error: unable to resolve hub RPC endpoint from hub.rpc_urls" >&2
  exit 1
fi

HUB_EID_VALUE=$(jq -r '(.hub // empty) | (.eid // "") | tostring' "$TOKENS_FILE")
if [[ -z "$HUB_EID_VALUE" || "$HUB_EID_VALUE" == "null" ]]; then
  echo "error: hub.eid missing from $TOKENS_FILE" >&2
  exit 1
fi

declare -a TOKEN_LABELS=()
declare -a VERIFIER_ADDRESSES=()
declare -a TOKEN_ADDRESSES=()
declare -a TOKEN_CHAIN_IDS=()
declare -a VERIFIER_EIDS=()
declare -a VERIFIER_RPCS=()
declare -a TOKEN_LEGACY_TX=()

token_count=0
while IFS=$'\t' read -r label verifier_addr token_addr chain_id rpc_url legacy_tx eid; do
  if [[ -z "$label" ]]; then
    echo "error: token entry missing label" >&2
    exit 1
  fi
  if [[ -z "$verifier_addr" ]]; then
    echo "error: token '$label' missing verifier_address" >&2
    exit 1
  fi
  if [[ -z "$token_addr" ]]; then
    echo "error: token '$label' missing token_address" >&2
    exit 1
  fi
  if [[ -z "$chain_id" ]]; then
    echo "error: token '$label' missing chain_id" >&2
    exit 1
  fi
  if [[ -z "$rpc_url" ]]; then
    echo "error: token '$label' must configure rpc_urls" >&2
    exit 1
  fi

  if [[ -z "$eid" || "$eid" == "null" ]]; then
    echo "error: token '$label' missing eid" >&2
    exit 1
  fi

  verifier_eid="$eid"

  TOKEN_LABELS+=("$label")
  VERIFIER_ADDRESSES+=("$verifier_addr")
  TOKEN_ADDRESSES+=("$token_addr")
  TOKEN_CHAIN_IDS+=("$chain_id")
  VERIFIER_EIDS+=("$verifier_eid")
  VERIFIER_RPCS+=("$rpc_url")
  TOKEN_LEGACY_TX+=("$legacy_tx")
  ((token_count++))
done < <(jq -r '.tokens[] |
  [
    (.label // ""),
    (.verifier_address // .verifierAddress // ""),
    (.token_address // .tokenAddress // ""),
    ((.chain_id // .chainId // "") | tostring),
    ((.rpc_urls // .rpcUrls // "") |
      if type == "array" then (if length > 0 then .[0] else "" end)
      elif type == "string" then .
      else "" end),
    (if (.legacy_tx // .legacyTx // false) then "true" else "false" end),
    ((.eid // "") | tostring)
  ] | @tsv' "$TOKENS_FILE")

if ((token_count == 0)); then
  echo "error: tokens array is empty in $TOKENS_FILE" >&2
  exit 1
fi

join_by_comma() {
  local IFS=","
  printf "%s" "$*"
}

VERIFIER_ADDRS_STR=$(join_by_comma "${VERIFIER_ADDRESSES[@]}")
TOKEN_ADDRS_STR=$(join_by_comma "${TOKEN_ADDRESSES[@]}")
TOKEN_CHAIN_IDS_STR=$(join_by_comma "${TOKEN_CHAIN_IDS[@]}")
VERIFIER_EIDS_STR=$(join_by_comma "${VERIFIER_EIDS[@]}")

echo "Running SetHubPeers against $HUB_RPC for ${#TOKEN_LABELS[@]} token(s)"
(
  cd "$SCRIPT_DIR"
  env \
    "HUB_ADDRESS=$HUB_ADDRESS" \
    "VERIFIER_ADDRESSES=$VERIFIER_ADDRS_STR" \
    "VERIFIER_EIDS=$VERIFIER_EIDS_STR" \
    "TOKEN_ADDRESSES=$TOKEN_ADDRS_STR" \
    "TOKEN_CHAIN_IDS=$TOKEN_CHAIN_IDS_STR" \
    forge script script/SetPeers.s.sol:SetHubPeers --rpc-url "$HUB_RPC" "${FORGE_ARGS[@]}"
)

for i in "${!TOKEN_LABELS[@]}"; do
  label="${TOKEN_LABELS[$i]}"
  verifier_addr="${VERIFIER_ADDRESSES[$i]}"
  verifier_rpc="${VERIFIER_RPCS[$i]}"

  legacy_flag="${TOKEN_LEGACY_TX[$i]}"
  legacy_flag_lower=$(printf '%s' "$legacy_flag" | tr '[:upper:]' '[:lower:]')
  if [[ "$legacy_flag_lower" == "true" ]]; then
    echo "Running SetVerifierPeers for '${label}' via $verifier_rpc (legacy tx)"
  else
    echo "Running SetVerifierPeers for '${label}' via $verifier_rpc"
  fi
  (
    cd "$SCRIPT_DIR"
    forge_args=("${FORGE_ARGS[@]}")
    if [[ "$legacy_flag_lower" == "true" ]]; then
      forge_args+=(--legacy)
    fi
    env \
      "HUB_ADDRESS=$HUB_ADDRESS" \
      "HUB_EID=$HUB_EID_VALUE" \
      "VERIFIER_ADDRESS=$verifier_addr" \
      forge script script/SetPeers.s.sol:SetVerifierPeers --rpc-url "$verifier_rpc" "${forge_args[@]}"
  )
done

if ((token_count > 1)); then
  peer_count=$((token_count - 1))
  echo "Running SetTokenPeers for ${token_count} chain(s); each token will set ${peer_count} peer(s)"
  for i in "${!TOKEN_LABELS[@]}"; do
    label="${TOKEN_LABELS[$i]}"
    token_addr="${TOKEN_ADDRESSES[$i]}"
    token_rpc="${VERIFIER_RPCS[$i]}"

    legacy_flag="${TOKEN_LEGACY_TX[$i]}"
    legacy_flag_lower=$(printf '%s' "$legacy_flag" | tr '[:upper:]' '[:lower:]')

    peer_eids=()
    peer_addrs=()
    for j in "${!TOKEN_LABELS[@]}"; do
      if [[ "$i" == "$j" ]]; then
        continue
      fi
      peer_eids+=("${VERIFIER_EIDS[$j]}")
      peer_addrs+=("${TOKEN_ADDRESSES[$j]}")
    done

    peer_eids_str=$(join_by_comma "${peer_eids[@]}")
    peer_addrs_str=$(join_by_comma "${peer_addrs[@]}")

    if [[ "$legacy_flag_lower" == "true" ]]; then
      echo "Running SetTokenPeers for '${label}' via $token_rpc (legacy tx)"
    else
      echo "Running SetTokenPeers for '${label}' via $token_rpc"
    fi
    (
      cd "$SCRIPT_DIR"
      forge_args=("${FORGE_ARGS[@]}")
      if [[ "$legacy_flag_lower" == "true" ]]; then
        forge_args+=(--legacy)
      fi
      env \
        "TOKEN_ADDRESS=$token_addr" \
        "PEER_ADDRESSES=$peer_addrs_str" \
        "PEER_EIDS=$peer_eids_str" \
        forge script script/SetPeers.s.sol:SetTokenPeers --rpc-url "$token_rpc" "${forge_args[@]}"
    )
  done
else
  echo "Skipping SetTokenPeers: only one token entry present"
fi

echo "SetPeers scripts completed"
