#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

SOURCE_DIR="${ROOT_DIR}/contracts/out"
TARGET_DIR="${ROOT_DIR}/client-common/abi"

declare -a CONTRACTS=("zERC20" "Verifier" "Hub" "Minter")

mkdir -p "${TARGET_DIR}"

for contract in "${CONTRACTS[@]}"; do
  source_path="${SOURCE_DIR}/${contract}.sol/${contract}.json"
  if [[ ! -f "${source_path}" ]]; then
    echo "Missing ABI: ${source_path}" >&2
    exit 1
  fi

  cp "${source_path}" "${TARGET_DIR}/${contract}.json"
done

echo "Copied ${#CONTRACTS[@]} ABIs to ${TARGET_DIR}"
