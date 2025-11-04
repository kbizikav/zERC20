# syntax=docker/dockerfile:1.7

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
    wasm \
    bin

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

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/workspace/target \
    cargo build --locked --release --package decider-prover \
    && cp target/release/decider-prover /workspace/bin/decider-prover

FROM debian:bullseye-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    libssl1.1 \
    zstd \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /workspace/bin/decider-prover /usr/local/bin/decider-prover
COPY docker/nova-artifacts.sh /usr/local/bin/nova-artifacts.sh
COPY docker/decider-prover-entrypoint.sh /usr/local/bin/decider-prover-entrypoint

ENV ARTIFACTS_DIR=/app/nova_artifacts \
    ARTIFACTS_SOURCE=volume \
    ARTIFACTS_STRIP_COMPONENTS=1 \
    RUST_LOG=info

RUN chmod +x /usr/local/bin/nova-artifacts.sh /usr/local/bin/decider-prover-entrypoint \
    && mkdir -p /app/nova_artifacts

VOLUME ["/app/nova_artifacts"]

EXPOSE 8081

ENTRYPOINT ["/usr/local/bin/decider-prover-entrypoint"]
