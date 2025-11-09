#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ABI_SRC="$REPO_ROOT/client-common/abi"
ABI_DEST="$REPO_ROOT/sdk/src/assets/abi"

WASM_SRC="$REPO_ROOT/wasm/pkg"
WASM_DEST="$REPO_ROOT/sdk/src/assets/wasm"

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

copy_assets "$ABI_SRC" "$ABI_DEST" "ABI files"
copy_assets "$WASM_SRC" "$WASM_DEST" "WASM pkg"

echo "SDK assets are up to date."
