#!/usr/bin/env python3
import argparse
import pathlib
import subprocess
import sys


def parse_args():
    parser = argparse.ArgumentParser(
        description="Run Groth16 Solana benchmark via send_verify CLI."
    )
    parser.add_argument(
        "--rpc-url",
        default="http://127.0.0.1:8899",
        help="Solana RPC URL",
    )
    parser.add_argument(
        "--program-id",
        required=True,
        help="Deployed program ID",
    )
    parser.add_argument(
        "--keypair",
        default=str(pathlib.Path.home() / ".config/solana/id.json"),
        help="Payer keypair path",
    )
    parser.add_argument(
        "--cu-limit",
        type=int,
        default=200000,
        help="Compute unit limit",
    )
    parser.add_argument(
        "--iterations",
        type=int,
        default=20,
        help="Number of iterations to run",
    )
    parser.add_argument(
        "--sleep-ms",
        type=int,
        default=0,
        help="Sleep between iterations (ms)",
    )
    parser.add_argument(
        "--proof-a-path",
        default="nova_artifacts/withdraw_local_groth16_precompile_proof_a.bin",
        help="Path to proof A (64 bytes)",
    )
    parser.add_argument(
        "--proof-b-path",
        default="nova_artifacts/withdraw_local_groth16_precompile_proof_b.bin",
        help="Path to proof B (128 bytes)",
    )
    parser.add_argument(
        "--proof-c-path",
        default="nova_artifacts/withdraw_local_groth16_precompile_proof_c.bin",
        help="Path to proof C (64 bytes)",
    )
    parser.add_argument(
        "--public-inputs-path",
        default="nova_artifacts/withdraw_local_groth16_precompile_public_inputs.bin",
        help="Path to public inputs (96 bytes)",
    )
    return parser.parse_args()


def main():
    args = parse_args()
    repo_root = pathlib.Path(__file__).resolve().parents[1]

    cmd = [
        "cargo",
        "run",
        "-p",
        "solana-groth16-program",
        "--features",
        "cli",
        "--bin",
        "send_verify",
        "--",
        "--rpc-url",
        args.rpc_url,
        "--program-id",
        args.program_id,
        "--keypair",
        args.keypair,
        "--proof-a-path",
        args.proof_a_path,
        "--proof-b-path",
        args.proof_b_path,
        "--proof-c-path",
        args.proof_c_path,
        "--public-inputs-path",
        args.public_inputs_path,
        "--cu-limit",
        str(args.cu_limit),
        "--iterations",
        str(args.iterations),
        "--sleep-ms",
        str(args.sleep_ms),
    ]

    result = subprocess.run(cmd, cwd=repo_root)
    return result.returncode


if __name__ == "__main__":
    sys.exit(main())
