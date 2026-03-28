# Multi-stage: build the hex-agent binary, then copy to minimal runtime image
FROM rust:1.86-slim AS builder

WORKDIR /build

# Copy workspace manifests
COPY Cargo.toml Cargo.lock ./
COPY hex-core/Cargo.toml hex-core/
COPY hex-agent/Cargo.toml hex-agent/
COPY hex-nexus/Cargo.toml hex-nexus/
COPY hex-cli/Cargo.toml hex-cli/
COPY hex-parser/Cargo.toml hex-parser/

# Create stub src files for dependency caching
RUN mkdir -p hex-core/src hex-agent/src hex-nexus/src hex-cli/src hex-parser/src \
    && echo 'fn main() {}' > hex-agent/src/main.rs \
    && echo '' > hex-core/src/lib.rs \
    && echo '' > hex-nexus/src/lib.rs \
    && echo '' > hex-cli/src/lib.rs \
    && echo '' > hex-parser/src/lib.rs

# Pre-build deps
RUN cargo build --release -p hex-agent 2>/dev/null || true

# Copy actual source
COPY hex-core/ hex-core/
COPY hex-agent/ hex-agent/
COPY hex-nexus/ hex-nexus/
COPY hex-cli/ hex-cli/
COPY hex-parser/ hex-parser/

# Build hex-agent
RUN cargo build --release -p hex-agent

# ── Runtime image ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    git \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/hex-agent /usr/local/bin/hex-agent

# Workspace is bind-mounted at runtime
WORKDIR /workspace

# Runtime env (overridden by docker run -e)
ENV NEXUS_HOST=host.docker.internal
ENV NEXUS_PORT=5555
ENV HEX_MCP_SERVER=0
ENV RUST_LOG=info

# Default: daemon mode (polls HexFlo tasks)
ENTRYPOINT ["hex-agent"]
CMD ["daemon"]
