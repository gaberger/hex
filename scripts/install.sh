#!/usr/bin/env bash
# Install hex binaries to /usr/local/bin via symlinks.
#
# Usage:
#   ./scripts/install.sh          # debug build (fast compile)
#   ./scripts/install.sh release  # release build (optimized)
#
# Requires sudo for /usr/local/bin access.

set -euo pipefail

PROFILE="${1:-debug}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="$ROOT/target/$PROFILE"
DEST="/usr/local/bin"

BINS=(hex hex-agent hex-nexus)

echo "⬡ hex install ($PROFILE)"
echo "  Root:   $ROOT"
echo "  Target: $TARGET"
echo "  Dest:   $DEST"
echo

# Build
if [ "$PROFILE" = "release" ]; then
    echo "  Building release..."
    cargo build --release -p hex-cli -p hex-agent -p hex-nexus
else
    echo "  Building debug..."
    cargo build -p hex-cli -p hex-agent -p hex-nexus
fi

echo

# Verify binaries exist
for bin in "${BINS[@]}"; do
    if [ ! -f "$TARGET/$bin" ]; then
        echo "  ✗ $bin not found at $TARGET/$bin"
        exit 1
    fi
done

# Create symlinks
echo "  Creating symlinks (may require password)..."
for bin in "${BINS[@]}"; do
    sudo ln -sf "$TARGET/$bin" "$DEST/$bin"
    echo "  ✓ $DEST/$bin → $TARGET/$bin"
done

echo
echo "⬡ Installed. Verify:"
for bin in "${BINS[@]}"; do
    VERSION=$("$DEST/$bin" --version 2>/dev/null || echo "ok")
    echo "  $bin: $VERSION"
done
