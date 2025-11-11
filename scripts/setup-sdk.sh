#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ABI_SRC="$REPO_ROOT/client-common/abi"
ABI_DEST="$REPO_ROOT/sdk/src/assets/abi"

WASM_SRC="$REPO_ROOT/wasm/pkg"
WASM_DEST="$REPO_ROOT/sdk/src/assets/wasm"

ARTIFACTS_SRC="$REPO_ROOT/nova_artifacts"
ARTIFACTS_DEST="$REPO_ROOT/sdk/src/assets/artifacts"

copy_assets() {
  local src="$1"
  local dest="$2"
  local label="$3"

  if [[ ! -d "$src" ]]; then
    echo "error: $label source not found at $src" >&2
    exit 1
  fi

  rm -rf "$dest"
  mkdir -p "$dest"
  cp -R "$src/." "$dest/"
  echo "✓ Copied $label to $dest"
}

copy_filtered_artifacts() {
  local src="$1"
  local dest="$2"

  if [[ ! -d "$src" ]]; then
    echo "error: Nova artifacts source not found at $src" >&2
    exit 1
  fi

  rm -rf "$dest"
  mkdir -p "$dest"

  # Skip decider-related binaries and Solidity sources; the SDK consumes only the Nova/Groth16 assets.
  rsync -a \
    --exclude '*decider*' \
    --exclude '*Decider*' \
    --exclude '*root_nova*' \
    --exclude '*.sol' \
    "$src/" "$dest/"

  echo "✓ Copied Nova artifacts (filtered) to $dest"
}

copy_assets "$ABI_SRC" "$ABI_DEST" "ABI files"
copy_assets "$WASM_SRC" "$WASM_DEST" "WASM pkg"
copy_filtered_artifacts "$ARTIFACTS_SRC" "$ARTIFACTS_DEST"

echo "SDK assets are up to date."
