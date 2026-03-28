# Multi-stage: build the hex-agent binary, then copy to minimal runtime image
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
COPY spacetime-modules/test-results/Cargo.toml spacetime-modules/test-results/

# Create stub src files for dependency caching
RUN mkdir -p hex-core/src hex-agent/src hex-nexus/src hex-cli/src hex-parser/src \
              hex-desktop/src spacetime-modules/test-results/src \
    && echo 'fn main() {}' > hex-agent/src/main.rs \
    && echo 'fn main() {}' > hex-cli/src/main.rs \
    && echo 'fn main() {}' > hex-desktop/src/main.rs \
    && echo '' > hex-core/src/lib.rs \
    && echo '' > hex-nexus/src/lib.rs \
    && echo '' > hex-cli/src/lib.rs \
    && echo '' > hex-parser/src/lib.rs \
    && echo '' > spacetime-modules/test-results/src/lib.rs

# Pre-build deps
RUN cargo build --release -p hex-agent 2>/dev/null || true

# Copy actual source
COPY hex-core/ hex-core/
COPY hex-agent/ hex-agent/
COPY hex-nexus/ hex-nexus/
COPY hex-cli/ hex-cli/
COPY hex-parser/ hex-parser/
COPY hex-desktop/ hex-desktop/
COPY spacetime-modules/test-results/ spacetime-modules/test-results/

# Touch all workspace source files so cargo invalidates the stub-built artifacts
RUN find hex-core hex-agent hex-nexus hex-cli hex-parser hex-desktop spacetime-modules \
        -name "*.rs" -exec touch {} \;

# Build hex-agent and hex-cli (agent uses hex CLI for project init + analysis)
RUN cargo build --release -p hex-agent -p hex-cli

# ── Runtime image ──────────────────────────────────────────────────────────
# Must use same glibc as builder (rust:1.93-slim is Debian trixie — glibc 2.39)
FROM rust:1.93-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    git \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/hex-agent /usr/local/bin/hex-agent
COPY --from=builder /build/target/release/hex /usr/local/bin/hex

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
