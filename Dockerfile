# syntax=docker/dockerfile:1.7
# Mirrors owilabs/audit-agent/orchestrator/Dockerfile (Rust toolchain + slim
# runtime + non-root user). cargo-chef is intentionally omitted: this image
# is built once for the hackathon, so the planner/cooker layer cache is not
# worth the extra ~20 lines.

ARG RUST_IMAGE=rust:1.94-slim-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

FROM ${RUST_IMAGE} AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo build --release --locked --bin brief-agent-buildos26

FROM ${RUNTIME_IMAGE} AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --system app && useradd --system --gid app app

WORKDIR /app

COPY --from=builder /app/target/release/brief-agent-buildos26 ./brief-agent-buildos26
COPY fixtures ./fixtures
COPY web ./web

USER app

EXPOSE 3030

HEALTHCHECK --interval=30s --timeout=3s CMD curl -f http://localhost:3030/health || exit 1

CMD ["./brief-agent-buildos26"]
