# Docs Project Environment Setup

Use this guide to prepare a local development workstation for contributing to the stealth announcement documentation and supporting code samples. The steps assume macOS or Linux. On Windows, use WSL2 with an Ubuntu distribution and follow the same instructions inside the Linux environment.

## 1. System Prerequisites

- Install build essentials (`build-essential`, `clang`, `cmake`, `pkg-config`, `git`, `openssl`, `libssl-dev`) via your OS package manager.
- Ensure OpenSSL headers are present (`brew install openssl@3` on macOS, `sudo apt install libssl-dev` on Ubuntu).
- Allocate at least 20 GB of free disk space for toolchains, replicas, and build artifacts.

## 2. Rust Toolchain

- Install rustup (https://rustup.rs) if it is not already present.
- Run `rustup toolchain install stable` and set it as default with `rustup default stable`.
- Add required targets:
  - `rustup target add wasm32-unknown-unknown`
  - `rustup target add wasm32-wasi`
- Add the following components: `rustup component add clippy rustfmt`.
- Verify with `rustc --version` and `cargo --version`.

## 3. Node.js & Package Managers

- Install Node.js 20 LTS or newer (via `nvm`, `asdf`, or the official installer).
- Ensure `npm` ≥ 10 is available. If the project standardizes on `pnpm` or `yarn`, install it globally.
- Install `wasm-pack` (`cargo install wasm-pack`) for building the WebAssembly helpers consumed by the TypeScript client.
- Optional: install `nvm` to simplify switching Node.js versions.

## 4. DFX CLI (Internet Computer SDK)

- Install the DFINITY dfx CLI: `sh -ci "$(curl -fsSL https://internetcomputer.org/install.sh)"`.
- After installation, pin the project’s required version (for example `DFX_VERSION=0.20.1 dfx upgrade`) if a `.dfx-version` file is present.
- Confirm the install with `dfx --version`.
- Enable the default identities and start a local replica once to let dfx download dependencies: `dfx start --background --clean --host 127.0.0.1:4943` then `dfx stop`.

## 5. PocketIC for Local Testing

- Download a PocketIC server release that matches your platform from the official repository’s Releases tab.
  - macOS: `pocket-ic-arm64-darwin.gz` (Apple silicon) or `pocket-ic-x86_64-darwin.gz` (Intel).
  - Linux: `pocket-ic-x86_64-linux.gz`; Linux containers on Apple silicon can use `pocket-ic-arm64-linux.gz`.
- Save the archive as `pocket-ic.gz`, decompress it, and make it executable:
  ```bash
  gzip -d pocket-ic.gz
  chmod +x pocket-ic
  ```
- Set `POCKET_IC_BIN` to the binary path (for example `export POCKET_IC_BIN="$(pwd)/pocket-ic"`). On macOS, clear the quarantine flag if prompted: `xattr -dr com.apple.quarantine pocket-ic`.
- Confirm the install with `"$POCKET_IC_BIN" --version` or `"$POCKET_IC_BIN" --help`.
- CI environments should cache the binary and export `POCKET_IC_BIN` before running integration tests.
