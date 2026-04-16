# Multi-stage: build the hex-nexus binary, then copy to minimal runtime image
FROM rust:1.93-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/* \
    && rustup component add rustfmt

# spacetimedb-lib build.rs calls `git rev-parse HEAD`; bypass by injecting commit hash
ENV SPACETIMEDB_NIX_BUILD_GIT_COMMIT=docker-build

WORKDIR /build

# Copy workspace manifests (all members to satisfy workspace resolver)
COPY Cargo.toml Cargo.lock ./
COPY hex-core/Cargo.toml hex-core/
COPY hex-agent/Cargo.toml hex-agent/
COPY hex-nexus/Cargo.toml hex-nexus/
COPY hex-cli/Cargo.toml hex-cli/
COPY hex-parser/Cargo.toml hex-parser/
COPY hex-desktop/Cargo.toml hex-desktop/
# Create stub src files for dependency caching
RUN mkdir -p hex-core/src hex-agent/src hex-nexus/src hex-cli/src hex-parser/src \
              hex-desktop/src \
    && echo 'fn main() {}' > hex-agent/src/main.rs \
    && echo 'fn main() {}' > hex-cli/src/main.rs \
    && echo 'fn main() {}' > hex-desktop/src/main.rs \
    && echo '' > hex-core/src/lib.rs \
    && echo '' > hex-nexus/src/lib.rs \
    && echo '' > hex-cli/src/lib.rs \
    && echo '' > hex-parser/src/lib.rs

# Pre-build deps
RUN cargo build --release -p hex-nexus 2>/dev/null || true

# Copy actual source
COPY hex-core/ hex-core/
COPY hex-agent/ hex-agent/
COPY hex-nexus/ hex-nexus/
COPY hex-cli/ hex-cli/
COPY hex-parser/ hex-parser/
COPY hex-desktop/ hex-desktop/
# Touch all workspace source files so cargo invalidates the stub-built artifacts
RUN find hex-core hex-agent hex-nexus hex-cli hex-parser hex-desktop \
        -name "*.rs" -exec touch {} \;

# Build hex-nexus
RUN cargo build --release -p hex-nexus

# ── Runtime image ──────────────────────────────────────────────────────────
FROM rust:1.93-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    git \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/hex-nexus /usr/local/bin/hex-nexus

# Workspace is bind-mounted at runtime
WORKDIR /workspace

# Runtime env
ENV RUST_LOG=info

ENTRYPOINT ["hex-nexus"]
