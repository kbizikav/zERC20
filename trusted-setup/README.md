# Trusted Setup

A Trusted Setup Ceremony toolkit for generating Groth16 and Nova/Decider parameters used in zERC20's ZK circuits.

## What is Trusted Setup?

Zero-knowledge proofs (especially Groth16) require pre-generated parameters for proving and verification. This parameter generation process is called "Trusted Setup."

For security, **as long as at least one participant is honest**, the entire system remains secure. Therefore, it's crucial that multiple independent participants contribute to the ceremony. Each participant uses secret random values to update the parameters and then destroys the secret values.

## Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        Ceremony Flow                            │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│   ┌────────────┐      ┌────────────┐      ┌────────────┐       │
│   │  PTAU File │ ──▶  │ Coordinator│ ──▶  │    S3      │       │
│   │  (Input)   │      │  (Server)  │      │ (Storage)  │       │
│   └────────────┘      └─────┬──────┘      └────────────┘       │
│                             │                                   │
│                             ▼                                   │
│   ┌──────────────────────────────────────────────────────┐     │
│   │              Participants (CLI)                       │     │
│   │  ┌─────────┐  ┌─────────┐  ┌─────────┐               │     │
│   │  │ User A  │  │ User B  │  │ User C  │  ...          │     │
│   │  └────┬────┘  └────┬────┘  └────┬────┘               │     │
│   │       │            │            │                     │     │
│   │       └────────────┴────────────┘                     │     │
│   │                    │                                  │     │
│   │                    ▼                                  │     │
│   │            Sequential Contributions                   │     │
│   └──────────────────────────────────────────────────────┘     │
│                             │                                   │
│                             ▼                                   │
│                    ┌────────────────┐                           │
│                    │   Finalize     │                           │
│                    │   (Artifacts)  │                           │
│                    └────────────────┘                           │
│                             │                                   │
│                             ▼                                   │
│     ┌─────────────────────────────────────────────────────┐    │
│     │  Output: Groth16 PK/VK, Nova params, Solidity code  │    │
│     └─────────────────────────────────────────────────────┘    │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

## Target Circuits

zERC20 requires Trusted Setup for the following 5 circuits:

| Circuit | Type | Purpose | Required PTAU |
|---------|------|---------|---------------|
| `withdraw_local` | Groth16 | Withdrawal proof from local transfer tree | power 14 (~16MB) |
| `withdraw_global` | Groth16 | Withdrawal proof from global transfer tree | power 14 (~16MB) |
| `decider_root` | Nova Decider | On-chain verification of Nova root proof | power 24 (~2.1GB) |
| `decider_withdraw_local` | Nova Decider | On-chain verification of local withdrawal Nova | power 24 (~2.1GB) |
| `decider_withdraw_global` | Nova Decider | On-chain verification of global withdrawal Nova | power 24 (~2.1GB) |

## Prerequisites

- **Rust** (1.70+ recommended)
- **AWS credentials** (if operating Coordinator)
  - S3 bucket access permissions
  - Environment variables `AWS_REGION`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` or IAM role
- **Sufficient disk space**
  - PTAU file for Decider circuits: ~2.1GB
  - Transcripts: Several hundred MB to several GB depending on circuit

## Components

### Coordinator (`trusted-setup/coordinator`)
Server that manages ceremonies. Built with Actix-web.

- Manages ceremony state with SQLite
- Stores transcripts in S3 (distributed to participants via presigned URLs)
- Prevents concurrent contributions with lease mechanism (default 15 minutes)
- Automatically cleans up expired leases in background

### CLI (`trusted-setup/cli`)
Command-line tool used by participants.

- Download and verify PTAU files
- Contribute to ceremonies
- Generate final artifacts (finalize)
- Check ceremony status

---

## Coordinator Operator Guide

### 1. Environment Setup

```bash
cd trusted-setup/coordinator
cp .env.example .env
```

Edit `.env` file with the following settings:

```bash
# Required: S3 bucket name
TRUSTED_SETUP_S3_BUCKET=your-bucket-name

# AWS authentication (set via environment variables or IAM role)
AWS_REGION=ap-northeast-1
AWS_ACCESS_KEY_ID=...
AWS_SECRET_ACCESS_KEY=...

# Optional: Lease TTL in seconds (default 900 = 15 minutes)
TRUSTED_SETUP_LEASE_TTL_SECONDS=900

# Optional: Listen address (default 0.0.0.0:8080)
TRUSTED_SETUP_COORDINATOR_ADDR=0.0.0.0:8080
```

### 2. Generate Initial Transcripts (Recommended)

To speed up ceremony initialization, it's recommended to pre-generate initial transcripts:

```bash
# Download PTAU files
cargo run --release -p trusted-setup-cli -- ptau download --power 14  # For Groth16
cargo run --release -p trusted-setup-cli -- ptau download --power 24  # For Decider

# Generate initial transcripts
cargo run --release -p trusted-setup-cli -- generate-initial-transcript --circuit withdraw_local
cargo run --release -p trusted-setup-cli -- generate-initial-transcript --circuit withdraw_global
cargo run --release -p trusted-setup-cli -- generate-initial-transcript --circuit decider_root
# ... repeat for other circuits
```

> **Note**: Generating initial transcripts for Decider circuits (`decider_root`, `decider_withdraw_local`, `decider_withdraw_global`) takes approximately **2 hours** each due to the large circuit size. Groth16 circuits complete much faster.

Generated files are saved to `~/.cache/zerc20/transcripts/`.

### 3. Start Coordinator

```bash
cargo run --release -p trusted-setup-coordinator
```

Server starts at `http://localhost:8080`.

### 4. Initialize Ceremony

```bash
# Example: Initialize ceremony for withdraw_local circuit
# Returns generated ceremony_id (UUID)
curl "http://localhost:8080/api/ceremonies/init/withdraw_local"
```

Response:
```json
{
  "ceremony_id": "550e8400-e29b-41d4-a716-446655440000",
  "step": 0,
  "transcript_key": "ceremonies/550e8400-e29b-41d4-a716-446655440000/transcripts/0.bin"
}
```

### 5. Check Ceremony Status

```bash
# List all ceremonies
curl http://localhost:8080/api/ceremonies

# Get specific ceremony status
curl http://localhost:8080/api/ceremonies/my-ceremony-001

# Get ceremony statistics
curl http://localhost:8080/api/ceremonies/my-ceremony-001/stats
```

---

## Participant Guide

### 1. Environment Setup

```bash
cd trusted-setup/cli
cp .env.example .env
```

Edit `.env` file:

```bash
# Coordinator URL
TRUSTED_SETUP_COORDINATOR_URL=http://localhost:8080

# Ceremony ID to participate in
TRUSTED_SETUP_CEREMONY_ID=my-ceremony-001

# Target circuit
TRUSTED_SETUP_CIRCUIT=withdraw_local
```

### 2. Download PTAU File

```bash
# Download appropriate PTAU based on circuit type
# For Groth16 (withdraw_local, withdraw_global):
cargo run --release -p trusted-setup-cli -- ptau download --power 14

# For Decider circuits:
cargo run --release -p trusted-setup-cli -- ptau download --power 24
```

File size is automatically verified after download.

### 3. Contribute to Ceremony

```bash
cargo run --release -p trusted-setup-cli -- contribute
```

This command performs the following:
1. Fetches current transcript from Coordinator
2. Computes contribution using random entropy
3. Uploads updated transcript
4. Verifies and finalizes contribution

**Note**: If interrupted with Ctrl+C, you can resume using `--state-file` option.

```bash
# Save state file while contributing
cargo run --release -p trusted-setup-cli -- contribute --state-file contribution.state

# Resume interrupted contribution
cargo run --release -p trusted-setup-cli -- resume --state-file contribution.state
```

### 4. Check Ceremony Status

```bash
cargo run --release -p trusted-setup-cli -- status
```

---

## Generating Artifacts (Finalize)

Once sufficient contributions have been collected, generate final parameters:

```bash
# Generate Groth16 parameters
cargo run --release -p trusted-setup-cli -- finalize \
  --circuit withdraw_local \
  --ceremony-id my-ceremony-001 \
  --public-base-url https://example.com/trusted-setup \
  --output-dir ./artifacts

# Generate Nova/Decider parameters
cargo run --release -p trusted-setup-cli -- finalize \
  --circuit decider_root \
  --ceremony-id my-ceremony-002 \
  --public-base-url https://example.com/trusted-setup \
  --output-dir ./artifacts
```

### Output Files

**For Groth16 circuits:**
- `{prefix}_groth16_pk.bin` - Proving key
- `{prefix}_groth16_vk.bin` - Verification key
- `{PascalPrefix}Groth16Verifier.sol` - Solidity verifier contract

**For Decider circuits:**
- `{prefix}_nova_pp.bin` - Nova prover parameters
- `{prefix}_nova_vp.bin` - Nova verification parameters
- `{prefix}_decider_pp.bin` - Decider prover parameters
- `{prefix}_decider_vp.bin` - Decider verification parameters
- `{PascalPrefix}NovaDecider.sol` - Solidity verifier contract

---

## CLI Commands

```bash
trusted-setup-cli <COMMAND>

Commands:
  ptau      PTAU-related subcommands
    download    Download PTAU file
    verify      Verify PTAU file hash

  contribute                Contribute to ceremony
  finalize                  Generate artifacts
  status                    Check ceremony/lease status
  resume                    Resume interrupted contribution
  generate-initial-transcript   Generate initial transcript
```

---

## File Paths

| Type | Default Path |
|------|--------------|
| PTAU (power 14) | `~/.cache/zerc20/ptau/ppot_0080_14.ptau` |
| PTAU (power 24) | `~/.cache/zerc20/ptau/ppot_0080_24.ptau` |
| Initial transcripts | `~/.cache/zerc20/transcripts/{circuit}_initial_transcript.bin` |
| PreparedAccumulator cache | `~/.cache/zerc20/prepared_accum/ppot_{power}_2pow{n}.bin` |
| SQLite database | `trusted-setup/coordinator/coordinator.sqlite` |

---

## Troubleshooting

### "active lease exists" error
Another participant is currently contributing. Wait until the lease expires (default 15 minutes) or contact the Coordinator operator.

### PTAU file size mismatch
Download may have been interrupted. Re-download with `--force` option:
```bash
cargo run --release -p trusted-setup-cli -- ptau download --power 24 --force
```

### "initial transcript not found" error
Initial transcript must be generated before Coordinator can initialize the ceremony:
```bash
cargo run --release -p trusted-setup-cli -- generate-initial-transcript --circuit <circuit>
```

---

## Security Notes

- **Never save or share** the entropy (random values) used during contribution
- It's recommended to restart your system after contributing to clear memory
- `--seed` option (deterministic contribution) should **only be used for testing**
