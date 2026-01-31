#!/usr/bin/env bash
set -euo pipefail

if [[ "${SKIP_MIGRATIONS:-0}" != "1" ]]; then
  echo "Running tree-indexer migrations..."
  sqlx migrate run --source /app/migrations
fi

exec tree-indexer "$@"
