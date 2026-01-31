# syntax=docker/dockerfile:1.7

# ============================================================
# Builder stage – compiles all workspace binaries in one pass
# ============================================================
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
    fee-manager \
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
COPY fee-manager/Cargo.toml fee-manager/Cargo.toml
COPY decider-prover/Cargo.toml decider-prover/Cargo.toml
COPY indexer/Cargo.toml indexer/Cargo.toml
COPY wasm/Cargo.toml wasm/Cargo.toml

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/workspace/target \
    cargo build --locked --release \
      --package zerc20-tree-indexer \
      --package zerc20-crosschain-job \
      --package zerc20-fee-manager \
      --package zerc20-decider-prover \
    && cp target/release/zerc20-tree-indexer      /workspace/bin/tree-indexer \
    && cp target/release/zerc20-crosschain-job    /workspace/bin/crosschain-job \
    && cp target/release/zerc20-fee-manager       /workspace/bin/fee-manager \
    && cp target/release/zerc20-decider-prover    /workspace/bin/decider-prover

RUN cargo install --locked sqlx-cli --no-default-features --features rustls,postgres

# ============================================================
# Runtime: indexer
# ============================================================
FROM debian:bullseye-slim AS indexer

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
    RUST_LOG=info

RUN chmod +x /usr/local/bin/indexer-entrypoint.sh /usr/local/bin/nova-artifacts.sh \
    && mkdir -p /app/nova_artifacts

VOLUME ["/app/nova_artifacts"]

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/indexer-entrypoint.sh"]

# ============================================================
# Runtime: crosschain-job
# ============================================================
FROM debian:bullseye-slim AS crosschain-job

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl1.1 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /workspace/bin/crosschain-job /usr/local/bin/crosschain-job

ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/crosschain-job"]

# ============================================================
# Runtime: fee-manager
# ============================================================
FROM debian:bullseye-slim AS fee-manager

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl1.1 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /workspace/bin/fee-manager /usr/local/bin/fee-manager

ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/fee-manager"]

# ============================================================
# Runtime: decider-prover
# ============================================================
FROM debian:bullseye-slim AS decider-prover

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
