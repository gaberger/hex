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
