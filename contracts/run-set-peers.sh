#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: run-set-peers.sh [--file PATH] [--metadata-file PATH] [--] [forge flags...]

Reads a tokens.json-formatted file, loads LayerZero endpoint metadata, and runs
SetPeers.s.sol (SetHubPeers once, SetVerifierPeers per token) with environment variables
derived from the configuration.

Options:
  --file PATH          Path to tokens.json (defaults to ../config/tokens.json)
  --metadata-file PATH LayerZero deployments JSON (defaults to layerzero_deployments.json)
  --help            Show this help message and exit
  --                Stop option parsing; following args are passed to forge script

Environment:
  PRIVATE_KEY       Required. Used by forge when broadcasting transactions.

Examples:
  ./run-set-peers.sh
  ./run-set-peers.sh --file ../config/tokens.prod.json -- --broadcast -vv
  # Defaults add '--broadcast --verify -vvvv' when no forge flags are provided
EOF
}

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT_DIR=$(cd "$SCRIPT_DIR/.." && pwd)
TOKENS_FILE="$ROOT_DIR/config/tokens.json"
METADATA_FILE="$SCRIPT_DIR/layerzero_deployments.json"
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
    --metadata-file)
      if (($# == 1)); then
        echo "error: --metadata-file expects a value" >&2
        exit 1
      fi
      METADATA_FILE="$2"
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

if [[ ! -f "$METADATA_FILE" ]]; then
  echo "error: LayerZero metadata file not found at $METADATA_FILE" >&2
  exit 1
fi
echo "Reading LayerZero metadata from $METADATA_FILE"
if ! LZ_METADATA=$(<"$METADATA_FILE"); then
  echo "error: failed to read LayerZero metadata from $METADATA_FILE" >&2
  exit 1
fi

fetch_eid_for_chain() {
  local chain_id=$1
  local matches=()
  while IFS=$'\t' read -r eid name env version; do
    if [[ -n "$eid" && "$eid" != "null" ]]; then
      matches+=("$eid|$name|$env|$version")
    fi
  done < <(
    jq -r --argjson target "$chain_id" '
      to_entries[]
      | .value as $cfg
      | ($cfg.deployments // [])[]
      | select(
          (try ($cfg.chainDetails.nativeChainId | tonumber) catch empty) == $target
          or (try ($cfg.chainDetails.native_chain_id | tonumber) catch empty) == $target
          or (try ($cfg.chainDetails.chainId | tonumber) catch empty) == $target
          or (try ($cfg.chainDetails.evmChainId | tonumber) catch empty) == $target
          or (try ($cfg.chainDetails.evm_chain_id | tonumber) catch empty) == $target
          or (try (.nativeChainId | tonumber) catch empty) == $target
          or (try (.native_chain_id | tonumber) catch empty) == $target
          or (try (.chainId | tonumber) catch empty) == $target
          or (try (.evmChainId | tonumber) catch empty) == $target
          or (try (.evm_chain_id | tonumber) catch empty) == $target
        )
      | [
          (.eid // .endpoint_id // .endpointId // .endpointIdV2 // empty),
          ($cfg.chainKey // ""),
          (.stage // $cfg.stage // $cfg.chainDetails.chainStatus // ""),
          ((.version // $cfg.version // null) | if . == null then "" else tostring end)
        ]
      | @tsv
    ' <<<"$LZ_METADATA"
  )

  if ((${#matches[@]} == 0)); then
    echo "error: no LayerZero endpoint id found for chain_id $chain_id" >&2
    return 1
  fi
  if ((${#matches[@]} > 1)); then
    local -a highest_matches=()
    local highest_version=-1
    for match in "${matches[@]}"; do
      IFS="|" read -r eid name env version <<<"$match"
      local version_num=0
      if [[ "$version" =~ ^[0-9]+$ ]]; then
        version_num=$version
      fi
      if ((version_num > highest_version)); then
        highest_version=$version_num
        highest_matches=("$match")
      elif ((version_num == highest_version)); then
        highest_matches+=("$match")
      fi
    done

    if ((${#highest_matches[@]} == 1)); then
      matches=("${highest_matches[0]}")
      IFS="|" read -r eid name env version <<<"${matches[0]}"
      echo "info: multiple LayerZero endpoints found for chain_id $chain_id; selecting eid=$eid (version=${version:-unknown}, name=${name:-unknown}, environment=${env:-unknown})" >&2
    else
      echo "error: multiple LayerZero endpoint ids found for chain_id $chain_id with the same highest version $highest_version:" >&2
      for match in "${highest_matches[@]}"; do
        IFS="|" read -r eid name env version <<<"$match"
        echo "  eid=$eid name=${name:-unknown} environment=${env:-unknown} version=${version:-unknown}" >&2
      done
      echo "Adjust the metadata source or tokens file to disambiguate." >&2
      return 1
    fi
  fi

  IFS="|" read -r eid _ _ _ <<<"${matches[0]}"
  if [[ -z "$eid" ]]; then
    echo "error: LayerZero metadata entry for chain_id $chain_id missing endpoint id" >&2
    return 1
  fi
  echo "$eid"
}

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

if ! HUB_EID_VALUE=$(fetch_eid_for_chain "$HUB_CHAIN_ID"); then
  echo "error: failed to resolve hub endpoint id" >&2
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
while IFS=$'\t' read -r label verifier_addr token_addr chain_id rpc_url legacy_tx; do
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

  if ! verifier_eid=$(fetch_eid_for_chain "$chain_id"); then
    echo "error: failed to resolve endpoint id for token '$label' (chain_id $chain_id)" >&2
    exit 1
  fi

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
    (if (.legacy_tx // .legacyTx // false) then "true" else "false" end)
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

echo "SetPeers scripts completed"
