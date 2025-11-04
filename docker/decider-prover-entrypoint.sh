#!/usr/bin/env bash
set -euo pipefail

. /usr/local/bin/nova-artifacts.sh

prepare_nova_artifacts

exec /usr/local/bin/decider-prover "$@"
