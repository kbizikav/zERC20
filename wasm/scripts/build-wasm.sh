#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WASM_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PKG_ROOT="$WASM_ROOT/pkg"

usage() {
  cat <<'EOF'
Usage: ./scripts/build-wasm.sh [web|node|all]

Build the zkERC20 wasm bindings for the desired target:
  web   - wasm-pack build --target web
  node  - wasm-pack build --target nodejs
  all   - build both targets (default)
EOF
}

build_target() {
  local label="$1"
  local target="$2"
  local out_dir="$PKG_ROOT/$label"

  rm -rf "$out_dir"

  echo "→ Building $label bindings (target=$target)"
  wasm-pack build \
    --release \
    --target "$target" \
    --out-dir "$out_dir" \
    --out-name "zkerc20_wasm" \
    "$WASM_ROOT"
}

ensure_pkg_root() {
  mkdir -p "$PKG_ROOT"
}

main() {
  local mode="${1:-all}"

  case "$mode" in
    web|node|all) ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown mode '$mode'"
      usage
      exit 1
      ;;
  esac

  ensure_pkg_root

  if [[ "$mode" == "web" || "$mode" == "all" ]]; then
    build_target "web" "web"
  fi

  if [[ "$mode" == "node" || "$mode" == "all" ]]; then
    build_target "node" "nodejs"
  fi

  echo "✓ wasm bindings built under $PKG_ROOT"
}

main "$@"
