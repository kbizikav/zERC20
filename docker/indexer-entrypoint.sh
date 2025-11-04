#!/usr/bin/env bash
set -euo pipefail

. /usr/local/bin/nova-artifacts.sh

prepare_nova_artifacts

if [[ "${SKIP_MIGRATIONS:-0}" != "1" ]]; then
  echo "Running tree-indexer migrations..."
  sqlx migrate run --source /app/migrations
fi

exec tree-indexer "$@"
