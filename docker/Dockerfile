# syntax=docker/dockerfile:1.7
# NOTE: This Dockerfile requires BuildKit (default since Docker 23.0).
#       If using an older version, set DOCKER_BUILDKIT=1 before building.

# ============================================================
# Builder stage – compiles all workspace binaries in one pass
# ============================================================
FROM rust:1.90-bookworm AS builder

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
    watcher \
    relay \
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
COPY watcher/Cargo.toml watcher/Cargo.toml
COPY relay/Cargo.toml relay/Cargo.toml

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/workspace/target \
    cargo build --locked --release \
      --package zerc20-tree-indexer \
      --package zerc20-crosschain-job \
      --package zerc20-fee-manager \
      --package zerc20-watcher \
      --package zerc20-relay \
    && cp target/release/zerc20-tree-indexer      /workspace/bin/tree-indexer \
    && cp target/release/zerc20-crosschain-job    /workspace/bin/crosschain-job \
    && cp target/release/zerc20-fee-manager       /workspace/bin/fee-manager \
    && cp target/release/zerc20-watcher           /workspace/bin/watcher \
    && cp target/release/zerc20-relay             /workspace/bin/relay

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo install --locked sqlx-cli --version 0.8.6 --no-default-features --features rustls,postgres

# ============================================================
# Runtime base – shared across all services
# ============================================================
FROM debian:bookworm-slim AS runtime-base

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -r -s /bin/false appuser

# ============================================================
# Runtime: indexer
# ============================================================
FROM runtime-base AS indexer

WORKDIR /app

COPY --from=builder /workspace/bin/tree-indexer /usr/local/bin/tree-indexer
COPY --from=builder /usr/local/cargo/bin/sqlx /usr/local/bin/sqlx

COPY indexer/migrations ./migrations
COPY docker/indexer-entrypoint.sh /usr/local/bin/indexer-entrypoint.sh

ENV RUST_LOG=info

RUN chmod +x /usr/local/bin/indexer-entrypoint.sh

USER appuser

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/indexer-entrypoint.sh"]

# ============================================================
# Runtime: crosschain-job
# ============================================================
FROM runtime-base AS crosschain-job

WORKDIR /app

COPY --from=builder /workspace/bin/crosschain-job /usr/local/bin/crosschain-job

ENV RUST_LOG=info

USER appuser

ENTRYPOINT ["/usr/local/bin/crosschain-job"]

# ============================================================
# Runtime: fee-manager
# ============================================================
FROM runtime-base AS fee-manager

WORKDIR /app

COPY --from=builder /workspace/bin/fee-manager /usr/local/bin/fee-manager

ENV RUST_LOG=info

USER appuser

ENTRYPOINT ["/usr/local/bin/fee-manager"]

# ============================================================
# Runtime: watcher
# ============================================================
FROM runtime-base AS watcher

WORKDIR /app

COPY --from=builder /workspace/bin/watcher /usr/local/bin/watcher

ENV RUST_LOG=info

USER appuser

ENTRYPOINT ["/usr/local/bin/watcher"]

# ============================================================
# Runtime: relay
# ============================================================
FROM runtime-base AS relay

WORKDIR /app

COPY --from=builder /workspace/bin/relay /usr/local/bin/relay

ENV RUST_LOG=info

USER appuser

EXPOSE 3000

ENTRYPOINT ["/usr/local/bin/relay"]
