#!/usr/bin/env bash
# hex-wrapper.sh — npm binary wrapper for Rust hex-cli
#
# When hex is installed via npm, this script delegates to the
# platform-specific Rust binary. Follows the esbuild/turbo pattern.
#
# Binary locations (checked in order):
#   1. ~/.hex/bin/hex         (installed by hex setup)
#   2. ./target/release/hex   (local cargo build)
#   3. Fallback: error with install instructions

set -euo pipefail

# Find the hex binary
find_hex_binary() {
    local hex_home="${HEX_HOME:-$HOME/.hex}"

    # 1. Installed binary
    if [ -x "$hex_home/bin/hex" ]; then
        echo "$hex_home/bin/hex"
        return
    fi

    # 2. Local cargo build (development)
    local script_dir="$(cd "$(dirname "$0")/.." && pwd)"
    if [ -x "$script_dir/target/release/hex" ]; then
        echo "$script_dir/target/release/hex"
        return
    fi
    if [ -x "$script_dir/target/debug/hex" ]; then
        echo "$script_dir/target/debug/hex"
        return
    fi

    # 3. Not found
    echo ""
}

HEX_BIN="$(find_hex_binary)"

if [ -z "$HEX_BIN" ]; then
    echo "Error: hex binary not found." >&2
    echo "" >&2
    echo "Install options:" >&2
    echo "  cargo install --path hex-cli    # Build from source" >&2
    echo "  hex setup                       # After npm install" >&2
    echo "" >&2
    echo "Or set HEX_HOME to the directory containing bin/hex" >&2
    exit 1
fi

exec "$HEX_BIN" "$@"
