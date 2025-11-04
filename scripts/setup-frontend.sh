#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
FRONTEND_DIR="$ROOT_DIR/frontend"

TOKENS_SRC="$ROOT_DIR/config/tokens.json"
ARTIFACTS_SRC_DIR="$ROOT_DIR/nova_artifacts"
WASM_SRC_DIR="$ROOT_DIR/wasm/pkg"
ABI_SRC_DIR="$ROOT_DIR/client-common/abi"

ARTIFACT_FILES=(
  withdraw_local_nova_pp.bin
  withdraw_local_nova_vp.bin
  withdraw_global_nova_pp.bin
  withdraw_global_nova_vp.bin
  withdraw_local_groth16_pk.bin
  withdraw_local_groth16_vk.bin
  withdraw_global_groth16_pk.bin
  withdraw_global_groth16_vk.bin
)

if [[ ! -d "$FRONTEND_DIR" ]]; then
  echo "frontend project not found at $FRONTEND_DIR" >&2
  exit 1
fi

embed_tokens_env() {
  local env_file="$FRONTEND_DIR/.env.local"
  if [[ ! -f "$TOKENS_SRC" ]]; then
    echo "Warning: $TOKENS_SRC not found; skipped embedding tokens config." >&2
    return
  fi

  if ! command -v gzip >/dev/null 2>&1; then
    echo "Error: gzip command not found; cannot compress tokens config." >&2
    exit 1
  fi

  if ! command -v base64 >/dev/null 2>&1; then
    echo "Error: base64 command not found; cannot encode tokens config." >&2
    exit 1
  fi

  local compressed
  compressed=$(gzip -c "$TOKENS_SRC" | base64 | tr -d '\n')
  local tmp
  tmp=$(mktemp)

  if [[ -f "$env_file" ]]; then
    grep -v '^VITE_TOKENS_COMPRESSED=' "$env_file" > "$tmp" || true
  fi

  {
    if [[ -f "$tmp" ]]; then
      cat "$tmp"
    fi
    echo "VITE_TOKENS_COMPRESSED=$compressed"
  } > "$env_file"

  rm -f "$tmp"
  echo "Embedded tokens configuration written to $env_file"
}

copy_artifacts() {
  local destination=$1
  mkdir -p "$destination"
  for file in "${ARTIFACT_FILES[@]}"; do
    local src="$ARTIFACTS_SRC_DIR/$file"
    if [[ -f "$src" ]]; then
      cp "$src" "$destination/$file"
    else
      echo "Warning: artifact $src not found" >&2
    fi
  done
  echo "Artifact binaries copied to $destination"
}

copy_wasm_bundle() {
  local destination=$1
  mkdir -p "$destination"
  if [[ -f "$WASM_SRC_DIR/zkerc20_wasm_bg.wasm" ]]; then
    cp "$WASM_SRC_DIR/zkerc20_wasm_bg.wasm" "$destination/zkerc20_wasm_bg.wasm"
    echo "Copied zkerc20_wasm_bg.wasm to $destination"
  else
    echo "Warning: wasm package not found in $WASM_SRC_DIR" >&2
  fi
  for extra in zkerc20_wasm.js zkerc20_wasm.d.ts zkerc20_wasm_bg.wasm.d.ts; do
    if [[ -f "$WASM_SRC_DIR/$extra" ]]; then
      cp "$WASM_SRC_DIR/$extra" "$destination/$extra"
    fi
  done
}

copy_abi_bundle() {
  local destination=$1
  mkdir -p "$destination"
  if compgen -G "$ABI_SRC_DIR/*.json" > /dev/null; then
    cp "$ABI_SRC_DIR"/*.json "$destination/"
    echo "Copied ABI json files to $destination"
  else
    echo "Warning: ABI sources not found in $ABI_SRC_DIR" >&2
  fi
}

PUBLIC_DIR="$FRONTEND_DIR/public"
ARTIFACTS_DIR="$PUBLIC_DIR/artifacts"

embed_tokens_env
copy_artifacts "$ARTIFACTS_DIR"

WASM_ASSET_DIR="$FRONTEND_DIR/src/assets/wasm"
ABI_ASSET_DIR="$FRONTEND_DIR/src/assets/abi"

copy_wasm_bundle "$WASM_ASSET_DIR"
copy_abi_bundle "$ABI_ASSET_DIR"

echo "Setup completed for frontend ($FRONTEND_DIR)"
