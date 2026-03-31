#!/usr/bin/env bash
# hex-install.sh — One-liner install for hex CLI
#
# Usage:
#   curl -sSL https://get.hex.dev | bash
#
# This script:
#   1. Detects OS and architecture
#   2. Downloads the latest hex binary (or uses local build)
#   3. Creates docker-compose.yml if docker is available
#   4. Runs hex doctor to verify installation
#
# Environment:
#   HEX_VERSION=26.4.0  # specific version (default: latest)
#   HEX_CHANNEL=stable   # stable|latest (default: stable)
#   HEX_SKIP_DOCKER=1   # skip docker-compose setup

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_err() { echo -e "${RED}[ERROR]${NC} $1" >&2; }

# Detect OS
detect_os() {
    case "$(uname -s)" in
        Linux*)     echo "linux" ;;
        Darwin*)    echo "macos" ;;
        MINGW*|MSYS*|CYGWIN*) echo "windows" ;;
        *)          echo "unknown" ;;
    esac
}

# Detect architecture
detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)    echo "x86_64" ;;
        aarch64|arm64)   echo "arm64" ;;
        *)               echo "unknown" ;;
    esac
}

# Download hex binary from GitHub releases
download_hex() {
    local version="${HEX_VERSION:-latest}"
    local os="$1"
    local arch="$2"
    local dest="$3"
    
    # For local development, use the built binary
    if [ -f "./target/release/hex" ]; then
        log_info "Using local build"
        cp "./target/release/hex" "$dest"
        chmod +x "$dest"
        return 0
    fi
    
    # GitHub releases URL (placeholder - update when releases exist)
    local base_url="https://github.com/gaberger/hex/releases"
    
    # Map to actual release artifact names
    local artifact_name=""
    case "${os}-${arch}" in
        linux-x86_64)   artifact_name="hex-x86_64-unknown-linux-gnu" ;;
        linux-arm64)    artifact_name="hex-aarch64-unknown-linux-gnu" ;;
        macos-x86_64)   artifact_name="hex-x86_64-apple-darwin" ;;
        macos-arm64)    artifact_name="hex-aarch64-apple-darwin" ;;
        *)              artifact_name="hex-${os}-${arch}" ;;
    esac
    
    # Default to known release if not specified
    if [ "$version" = "latest" ]; then
        version="26.6.0"
    fi
    
    local url="$base_url/download/v${version}/$artifact_name"
    
    log_info "Downloading hex from $url"
    if curl -sSLf -L "$url" -o "$dest" 2>/dev/null; then
        chmod +x "$dest"
        return 0
    fi
    
    # Fallback: build from source (if cargo available)
    if command -v cargo &> /dev/null; then
        log_warn "Download failed, building from source..."
        cargo build --release -p hex-cli
        cp "./target/release/hex" "$dest"
        chmod +x "$dest"
        return 0
    fi
    
    return 1
}

# Setup docker-compose for dependencies
setup_docker() {
    if [ "${HEX_SKIP_DOCKER:-0}" = "1" ]; then
        log_info "Skipping docker setup (HEX_SKIP_DOCKER=1)"
        return 0
    fi
    
    if ! command -v docker &> /dev/null; then
        log_warn "Docker not found, skipping docker-compose setup"
        return 0
    fi
    
    if ! docker info &> /dev/null; then
        log_warn "Docker not running, skipping docker-compose setup"
        return 0
    fi
    
    # Create docker-compose.yml if it doesn't exist
    if [ ! -f "docker-compose.yml" ]; then
        log_info "Creating docker-compose.yml for hex dependencies..."
        cat > docker-compose.yml << 'EOF'
version: '3.8'

services:
  spacetimedb:
    image: clockworklabs/spacetime:0.12.1
    ports:
      - "3033:3033"
    volumes:
      - spacetimedb_data:/data
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:3033/v1/ping"]
      interval: 10s
      timeout: 5s
      retries: 5

  hex-nexus:
    build:
      context: .
      dockerfile: Dockerfile.nexus
    ports:
      - "5555:5555"
    depends_on:
      spacetimedb:
        condition: service_healthy
    environment:
      - HEX_SPACETIMEDB_HOST=http://spacetimedb:3033
      - RUST_LOG=info

volumes:
  spacetimedb_data:
EOF
        log_info "Created docker-compose.yml"
    else
        log_info "docker-compose.yml already exists"
    fi
    
    return 0
}

# Main
main() {
    local os detect_os
    local arch detect_arch
    local install_dir="${HOME}/.hex/bin"
    
    os=$(detect_os)
    arch=$(detect_arch)
    
    echo "⬡ hex installer"
    echo "  OS:   $os"
    echo "  Arch: $arch"
    echo
    
    # Create install directory
    mkdir -p "$install_dir"
    
    # Download binary
    local hex_bin="$install_dir/hex"
    if ! download_hex "$os" "$arch" "$hex_bin"; then
        log_err "Failed to download hex binary"
        exit 1
    fi
    
    # Add to PATH
    local shell_rc="${HOME}/.bashrc"
    if [ "$(uname -s)" = "Darwin" ]; then
        shell_rc="${HOME}/.zshrc"
    fi
    
    if ! grep -q "$install_dir" "$shell_rc" 2>/dev/null; then
        echo "export PATH=\"\$PATH:$install_dir\"" >> "$shell_rc"
        log_info "Added $install_dir to PATH (source ~/.bashrc or ~/.zshrc)"
    fi
    
    # Setup docker if available
    setup_docker
    
    # Verify installation
    echo
    log_info "Verifying installation..."
    if command -v hex &> /dev/null; then
        hex doctor || log_warn "hex doctor had issues (may need to start hex-nexus)"
    else
        log_warn "hex not in PATH, run: source ~/.bashrc"
    fi
    
    echo
    log_info "Installation complete!"
}

main "$@"