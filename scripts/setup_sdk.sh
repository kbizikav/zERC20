#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ABI_SRC="$REPO_ROOT/client-common/abi"
ABI_DEST="$REPO_ROOT/sdk/src/assets/abi"

WASM_CRATE="$REPO_ROOT/wasm"
WASM_SRC="$WASM_CRATE/pkg"
WASM_DEST="$REPO_ROOT/sdk/src/assets/wasm"

ARTIFACTS_SRC="$REPO_ROOT/nova_artifacts"
ARTIFACTS_DEST="$REPO_ROOT/sdk/src/assets/artifacts"

ensure_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command not found: $1" >&2
    exit 1
  fi
}

build_wasm_target() {
  local target="$1"
  local out_dir="$2"

  rm -rf "$WASM_CRATE/$out_dir"
  (
    cd "$WASM_CRATE"
    wasm-pack build --release --target "$target" --out-dir "$out_dir"
  )
  echo "✓ Built WASM target '$target' to $WASM_CRATE/$out_dir"
}

build_wasm() {
  ensure_command wasm-pack
  echo "Building WASM bundles..."
  build_wasm_target web "pkg/web"
  build_wasm_target nodejs "pkg/node"
}

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

build_wasm
copy_assets "$ABI_SRC" "$ABI_DEST" "ABI files"
copy_assets "$WASM_SRC" "$WASM_DEST" "WASM pkg"
copy_filtered_artifacts "$ARTIFACTS_SRC" "$ARTIFACTS_DEST"

echo "SDK assets are up to date."
