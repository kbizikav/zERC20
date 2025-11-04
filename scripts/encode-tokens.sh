#!/usr/bin/env bash
set -euo pipefail

show_usage() {
  cat <<'USAGE'
Usage: scripts/encode-tokens.sh [path/to/tokens.json]

Reads the tokens.json file (defaults to config/tokens.json), compresses it with gzip,
encodes it with base64, and prints the value suitable for the VITE_TOKENS_COMPRESSED env var.

Example:
  VITE_TOKENS_COMPRESSED="$(scripts/encode-tokens.sh)"
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  show_usage
  exit 0
fi

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
TOKENS_FILE="${1:-$ROOT_DIR/config/tokens.json}"

if [[ ! -f "$TOKENS_FILE" ]]; then
  echo "Error: tokens file not found at $TOKENS_FILE" >&2
  exit 1
fi

if ! command -v gzip >/dev/null 2>&1; then
  echo "Error: gzip command not found; cannot compress tokens config." >&2
  exit 1
fi

if ! command -v base64 >/dev/null 2>&1; then
  echo "Error: base64 command not found; cannot encode tokens config." >&2
  exit 1
fi

gzip -c "$TOKENS_FILE" | base64 | tr -d '\n'
echo
