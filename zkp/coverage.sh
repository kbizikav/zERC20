#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
output_dir="${repo_root}/zkp/coverage/html"

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "cargo-llvm-cov is not installed. Run: cargo install cargo-llvm-cov" >&2
  exit 1
fi

mkdir -p "${output_dir}"

cd "${repo_root}"
cargo llvm-cov -p zkp --html --output-dir "${output_dir}"
