#!/bin/bash
set -e

VERSION=${1:-latest}
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# Resolve target triple
if [ "$OS" = "darwin" ] && [ "$ARCH" = "arm64" ]; then
  TARGET="aarch64-apple-darwin"
elif [ "$OS" = "linux" ] && [ "$ARCH" = "x86_64" ]; then
  TARGET="x86_64-unknown-linux-gnu"
else
  echo "Unsupported platform: $OS/$ARCH"
  exit 1
fi

# Resolve install prefix
# On immutable Linux distros (Bazzite, Silverblue, uBlue) /usr/local/bin is
# read-only. Default to ~/.local/bin which is always user-writable.
# Override with: INSTALL_PREFIX=/usr/local ./install.sh
if [ -n "${INSTALL_PREFIX:-}" ]; then
  BIN_DIR="$INSTALL_PREFIX/bin"
elif [ "$OS" = "linux" ]; then
  BIN_DIR="${HOME}/.local/bin"
else
  BIN_DIR="/usr/local/bin"
fi

# Get latest version tag if not specified
if [ "$VERSION" = "latest" ]; then
  VERSION=$(curl -s https://api.github.com/repos/gaberger/hex/releases/latest | grep tag_name | cut -d'"' -f4)
  VERSION="${VERSION#v}"  # strip leading 'v' if present
fi

URL="https://github.com/gaberger/hex/releases/download/v${VERSION}/hex-${VERSION}-${TARGET}.tar.gz"
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

echo "Downloading hex ${VERSION} for ${TARGET}..."
curl -fSL "$URL" | tar xz -C "$TMPDIR"

mkdir -p "$BIN_DIR"
if [ -w "$BIN_DIR" ]; then
  mv "$TMPDIR/hex" "$BIN_DIR/hex"
  mv "$TMPDIR/hex-nexus" "$BIN_DIR/hex-nexus"
else
  sudo mv "$TMPDIR/hex" "$BIN_DIR/hex"
  sudo mv "$TMPDIR/hex-nexus" "$BIN_DIR/hex-nexus"
fi

echo "hex ${VERSION} installed to ${BIN_DIR}"

# Remind the user if ~/.local/bin is not in PATH
if [ "$BIN_DIR" = "${HOME}/.local/bin" ]; then
  case ":${PATH}:" in
    *":${BIN_DIR}:"*) ;;
    *) echo "  NOTE: add ${BIN_DIR} to your PATH (e.g. add to ~/.bashrc: export PATH=\"\$HOME/.local/bin:\$PATH\")" ;;
  esac
fi

"$BIN_DIR/hex" --version

# ── SpacetimeDB ──────────────────────────────────────────────────────────
# hex requires SpacetimeDB. Install if not present.
if command -v spacetime >/dev/null 2>&1; then
  echo "SpacetimeDB: $(spacetime version 2>/dev/null || echo 'already installed')"
else
  echo ""
  echo "Installing SpacetimeDB..."
  STDB_VERSION=$(curl -sI https://github.com/clockworklabs/SpacetimeDB/releases/latest | grep -i location | sed 's|.*/tag/||;s/\r//')
  STDB_TARGET=""
  if [ "$OS" = "darwin" ] && [ "$ARCH" = "arm64" ]; then
    STDB_TARGET="aarch64-apple-darwin"
  elif [ "$OS" = "linux" ] && [ "$ARCH" = "x86_64" ]; then
    STDB_TARGET="x86_64-unknown-linux-gnu"
  fi

  if [ -n "$STDB_TARGET" ]; then
    STDB_URL="https://github.com/clockworklabs/SpacetimeDB/releases/download/${STDB_VERSION}/spacetime-${STDB_TARGET}.tar.gz"
    STDB_TMP=$(mktemp -d)
    curl -fSL "$STDB_URL" | tar xz -C "$STDB_TMP"

    if [ -w "$BIN_DIR" ]; then
      mv "$STDB_TMP/spacetimedb-cli" "$BIN_DIR/spacetime"
      mv "$STDB_TMP/spacetimedb-standalone" "$BIN_DIR/spacetimedb-standalone"
    else
      sudo mv "$STDB_TMP/spacetimedb-cli" "$BIN_DIR/spacetime"
      sudo mv "$STDB_TMP/spacetimedb-standalone" "$BIN_DIR/spacetimedb-standalone"
    fi
    chmod +x "$BIN_DIR/spacetime" "$BIN_DIR/spacetimedb-standalone"
    rm -rf "$STDB_TMP"

    # Generate JWT keys if missing
    STDB_CONFIG="${HOME}/.config/spacetime"
    if [ ! -f "$STDB_CONFIG/id_ecdsa" ]; then
      mkdir -p "$STDB_CONFIG"
      openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:prime256v1 \
        -out "$STDB_CONFIG/id_ecdsa" 2>/dev/null
      openssl pkey -in "$STDB_CONFIG/id_ecdsa" -pubout \
        -out "$STDB_CONFIG/id_ecdsa.pub" 2>/dev/null
    fi

    echo "SpacetimeDB installed to ${BIN_DIR}"
  else
    echo "  WARN: SpacetimeDB not available for ${OS}/${ARCH} — install manually"
  fi
fi

echo ""
echo "Setup complete. Quick start:"
echo "  spacetimedb-standalone start --data-dir ~/.local/share/spacetime/data --jwt-key-dir ~/.config/spacetime --listen-addr 127.0.0.1:3033 &"
echo "  hex stdb hydrate        # publish WASM modules"
echo "  hex nexus start         # start hex-nexus"
echo "  cd your-project && hex init"
