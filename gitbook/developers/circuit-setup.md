# Circuit Setup

This guide covers downloading and testing zERC20 circuit artifacts using the `zerc20-circuit-setup` CLI tool.

## Why You Need Circuit Artifacts

Circuit artifacts are required for:

- **Running the Indexer**: The indexer uses these artifacts to generate Nova proofs for transfer root transitions
- **Running the Decider Prover**: The decider-prover uses these artifacts to finalize Nova proofs into Decider proofs for on-chain verification
- **CLI Proof Generation**: The CLI uses these artifacts to generate withdrawal proofs (both Nova and Groth16)

Without these artifacts, you cannot run the indexer, decider-prover, or generate proofs using the CLI.

## Official Manifest Digests

Always verify that the manifest digest displayed during `download` matches the official value.

| Version | Manifest Digest |
|---------|-----------------|
| `1.0.0` | `f8181f89d502cd5bebc4445c4305c6c692f92deb202a18ce5d7c41694b10a7a4` |

## Installation

```bash
# Clone the repository
git clone https://github.com/InternetMaximalism/zerc20.git
cd zerc20

# Install the CLI
cargo install --path circuit-setup
```

## Configuration

Set environment variables or use a `.env` file.

```bash
# Copy the example environment file
cp circuit-setup/.env.example circuit-setup/.env
```

Contents of `.env.example`:

```bash
# Artifact version
ARTIFACTS_VERSION=1.0.0

# Local directory for circuit artifacts
NOVA_ARTIFACTS_DIR=../nova_artifacts

# Base URL for download (public HTTP/HTTPS)
ARTIFACTS_BASE_URL=https://zerc20-prod-public-uploads.s3.ap-southeast-1.amazonaws.com/circuit-setup

# Logging level (error, warn, info, debug, trace)
RUST_LOG=info
```

## Commands

### Download

Download circuit artifacts from a public URL and automatically verify SHA256 hashes.

```bash
zerc20-circuit-setup download --version 1.0.0
```

Or specify the URL explicitly:

```bash
zerc20-circuit-setup download \
  --version 1.0.0 \
  --base-url https://zerc20-prod-public-uploads.s3.ap-southeast-1.amazonaws.com/circuit-setup \
  --artifacts-dir ./nova_artifacts
```

| Option | Environment Variable | Description |
|--------|---------------------|-------------|
| `--version` | `ARTIFACTS_VERSION` | Version to download |
| `--base-url` | `ARTIFACTS_BASE_URL` | Public URL for artifacts |
| `--artifacts-dir` | `NOVA_ARTIFACTS_DIR` | Local output directory |

### Test

Test downloaded artifacts by generating and verifying dummy proofs.

```bash
zerc20-circuit-setup test --artifacts-dir ./nova_artifacts
```

## More Information

See the [README](https://github.com/InternetMaximalism/zerc20/blob/main/circuit-setup/README.md) for full usage instructions.
