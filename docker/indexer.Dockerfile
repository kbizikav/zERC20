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

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/workspace/target \
    cargo build --locked --release --package tree-indexer \
    && cp target/release/tree-indexer /workspace/bin/tree-indexer

RUN cargo install --locked sqlx-cli --no-default-features --features rustls,postgres

FROM debian:bullseye-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl1.1 \
    curl \
    zstd \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /workspace/bin/tree-indexer /usr/local/bin/tree-indexer
COPY --from=builder /usr/local/cargo/bin/sqlx /usr/local/bin/sqlx

COPY indexer/migrations ./migrations
COPY docker/nova-artifacts.sh /usr/local/bin/nova-artifacts.sh
COPY docker/indexer-entrypoint.sh /usr/local/bin/indexer-entrypoint.sh

ENV ARTIFACTS_DIR=/app/nova_artifacts \
    ARTIFACTS_SOURCE=volume \
    ARTIFACTS_STRIP_COMPONENTS=1 \
    TOKENS_FILE_PATH=/config/tokens.json \
    INDEXER_HTTP_ADDR=0.0.0.0:8081 \
    RUST_LOG=info

RUN chmod +x /usr/local/bin/indexer-entrypoint.sh /usr/local/bin/nova-artifacts.sh \
    && mkdir -p /app/nova_artifacts

VOLUME ["/app/nova_artifacts"]

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/indexer-entrypoint.sh"]
