#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

SRC_DIR="${REPO_ROOT}/nova_artifacts"
DST_DIR="${REPO_ROOT}/contracts/src/verifiers"

if [[ ! -d "${SRC_DIR}" ]]; then
  echo "Source directory ${SRC_DIR} does not exist. Generate artifacts first." >&2
  exit 1
fi

mkdir -p "${DST_DIR}"

shopt -s nullglob
verifier_files=("${SRC_DIR}"/*.sol)
shopt -u nullglob

if [[ ${#verifier_files[@]} -eq 0 ]]; then
  echo "No verifier contracts found in ${SRC_DIR}." >&2
  exit 1
fi

# Remove any previously copied verifier contracts to avoid stale files.
find "${DST_DIR}" -type f -name "*.sol" -delete 2>/dev/null || true

for file in "${verifier_files[@]}"; do
  cp "${file}" "${DST_DIR}/"
  echo "Copied $(basename "${file}") to ${DST_DIR}"
done
