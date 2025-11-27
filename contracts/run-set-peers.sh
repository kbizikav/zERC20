#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PYTHON_SCRIPT="$SCRIPT_DIR/run_set_peers.py"

if [[ ! -f "$PYTHON_SCRIPT" ]]; then
  echo "error: helper script not found at $PYTHON_SCRIPT" >&2
  exit 1
fi

exec python3 "$PYTHON_SCRIPT" "$@"
