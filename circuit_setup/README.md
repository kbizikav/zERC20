# zerc20-circuit-setup

CLI tool for managing zERC20 circuit artifacts (generation, S3 upload/download, and Solidity verifier generation).

## Installation

```bash
cargo install --path circuit_setup
```

## Quick Start

```bash
# Copy and configure environment
cp circuit_setup/.env.example circuit_setup/.env

# Generate artifacts
zerc20-circuit-setup generate --version 1.0.0

# Upload to S3
zerc20-circuit-setup upload --bucket my-bucket

# Download from public URL
zerc20-circuit-setup download --version 1.0.0 --base-url https://bucket.s3.amazonaws.com/prefix

# Generate Solidity verifiers
zerc20-circuit-setup generate-verifier
```

## Commands

### `generate`

Generate all circuit artifacts (Nova params, Decider params, Groth16 keys).

```bash
zerc20-circuit-setup generate --version <VERSION> [--seed <SEED>] [--artifacts-dir <DIR>]
```

| Option | Env Variable | Description |
|--------|--------------|-------------|
| `--version` | `ARTIFACTS_VERSION` | Artifact version (required) |
| `--seed` | - | Fixed seed for deterministic generation (**not recommended for production**) |
| `--artifacts-dir` | `NOVA_ARTIFACTS_DIR` | Output directory (default: `../nova_artifacts`) |

**Output files:**
- `root_nova_pp.bin`, `root_nova_vp.bin`, `root_decider_pp.bin`, `root_decider_vp.bin`
- `withdraw_local_*.bin` (nova, decider, groth16)
- `withdraw_global_*.bin` (nova, decider, groth16)
- `manifest.json`

### `upload`

Upload artifacts to S3. Version is automatically read from `manifest.json`. Files larger than 1GB are automatically uploaded using multipart upload.

```bash
zerc20-circuit-setup upload --bucket <BUCKET> [--prefix <PREFIX>]
```

| Option | Env Variable | Description |
|--------|--------------|-------------|
| `--bucket` | `S3_BUCKET` | S3 bucket name |
| `--prefix` | `S3_PREFIX` | S3 key prefix (optional) |
| `--artifacts-dir` | `NOVA_ARTIFACTS_DIR` | Local artifacts directory |

**S3 structure:**
```
s3://<bucket>/<prefix>/<version>/
├── manifest.json
├── root_nova_pp.bin
├── root_nova_vp.bin
├── ...
└── withdraw_global_groth16_vk.bin
```

### `download`

Download artifacts from a public URL and verify SHA256 hashes. No AWS credentials required.

```bash
zerc20-circuit-setup download --version <VERSION> --base-url <URL>
```

| Option | Env Variable | Description |
|--------|--------------|-------------|
| `--version` | `ARTIFACTS_VERSION` | Artifact version to download |
| `--base-url` | `ARTIFACTS_BASE_URL` | Base URL (e.g., `https://bucket.s3.amazonaws.com/prefix`) |
| `--artifacts-dir` | `NOVA_ARTIFACTS_DIR` | Local output directory |

**Features:**
- Downloads `manifest.json` first
- Verifies each file's SHA256 hash against manifest
- Displays manifest digest for verification
- Works with any public HTTP(S) URL

### `generate-verifier`

Generate Solidity verifier contracts from existing artifacts.

```bash
zerc20-circuit-setup generate-verifier [--artifacts-dir <DIR>] [--output <DIR>]
```

| Option | Env Variable | Description |
|--------|--------------|-------------|
| `--artifacts-dir` | `NOVA_ARTIFACTS_DIR` | Input artifacts directory |
| `--output`, `-o` | - | Output directory for .sol files (defaults to artifacts_dir) |

**Output files:**
- `RootNovaDecider.sol`
- `WithdrawLocalNovaDecider.sol`
- `WithdrawGlobalNovaDecider.sol`
- `WithdrawLocalGroth16Verifier.sol`
- `WithdrawGlobalGroth16Verifier.sol`

## Environment Variables

| Variable | Description |
|----------|-------------|
| `ARTIFACTS_VERSION` | Artifact version |
| `NOVA_ARTIFACTS_DIR` | Local artifacts directory |
| `S3_BUCKET` | S3 bucket name (for upload) |
| `S3_PREFIX` | S3 key prefix (for upload) |
| `ARTIFACTS_BASE_URL` | Base URL for download |
| `RUST_LOG` | Log level (`error`, `warn`, `info`, `debug`, `trace`) |

## manifest.json

The manifest contains SHA256 hashes for all artifacts:

```json
{
  "version": "1.0.0",
  "created_at": "2026-01-28T12:00:00Z",
  "circuits": {
    "root": {
      "nova_pp": { "path": "1.0.0/root_nova_pp.bin", "sha256": "abc123..." },
      "nova_vp": { "path": "1.0.0/root_nova_vp.bin", "sha256": "def456..." },
      "decider_pp": { "path": "1.0.0/root_decider_pp.bin", "sha256": "..." },
      "decider_vp": { "path": "1.0.0/root_decider_vp.bin", "sha256": "..." }
    },
    "withdraw_local": { ... },
    "withdraw_global": { ... }
  }
}
```

**Manifest digest:** Computed using canonical JSON (RFC 8785) + SHA256.

## AWS Credentials

This tool uses the standard AWS credential chain. Configure credentials using one of:

- Environment variables (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`)
- AWS credentials file (`~/.aws/credentials`)
- IAM role (for EC2/ECS)

## Examples

### Production workflow

```bash
# 1. Generate with secure entropy
RUST_LOG=info zerc20-circuit-setup generate --version 1.0.0

# 2. Upload to S3 (version from manifest.json)
zerc20-circuit-setup upload --bucket zerc20-artifacts --prefix prod

# 3. Generate Solidity verifiers
zerc20-circuit-setup generate-verifier
```

### Development workflow (deterministic)

```bash
# Generate with fixed seed (for reproducible testing)
zerc20-circuit-setup generate --version dev --seed 42
```

### Download and verify

```bash
# Download artifacts from public URL
zerc20-circuit-setup download --version 1.0.0 --base-url https://zerc20-artifacts.s3.amazonaws.com/prod

# Output shows:
# - Manifest digest (for verification)
# - Hash verification status for each file
```
