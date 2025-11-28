FROM rust:1.90-bullseye AS builder

WORKDIR /workspace

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    clang \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p \
    api-types \
    client-common \
    cli \
    zkp \
    crosschain-job \
    decider-prover \
    indexer \
    wasm

COPY Cargo.toml Cargo.lock ./
COPY api-types/Cargo.toml api-types/Cargo.toml
COPY client-common/Cargo.toml client-common/Cargo.toml
COPY cli/Cargo.toml cli/Cargo.toml
COPY zkp/Cargo.toml zkp/Cargo.toml
COPY crosschain-job/Cargo.toml crosschain-job/Cargo.toml
COPY decider-prover/Cargo.toml decider-prover/Cargo.toml
COPY indexer/Cargo.toml indexer/Cargo.toml
COPY wasm/Cargo.toml wasm/Cargo.toml

COPY . .

RUN cargo build --locked --release --package crosschain-job

FROM debian:bullseye-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl1.1 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /workspace/target/release/crosschain-job /usr/local/bin/crosschain-job

ENV TOKENS_FILE_PATH=/config/tokens.json \
    RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/crosschain-job"]
