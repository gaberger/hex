#!/bin/bash
set -e
VERSION=${1:-latest}
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# Resolve target
if [ "$OS" = "darwin" ] && [ "$ARCH" = "arm64" ]; then
  TARGET="aarch64-apple-darwin"
elif [ "$OS" = "linux" ] && [ "$ARCH" = "x86_64" ]; then
  TARGET="x86_64-unknown-linux-gnu"
else
  echo "Unsupported platform: $OS/$ARCH"
  exit 1
fi

# Get latest version if not specified
if [ "$VERSION" = "latest" ]; then
  VERSION=$(curl -s https://api.github.com/repos/gaberger/hex/releases/latest | grep tag_name | cut -d'"' -f4)
fi

URL="https://github.com/gaberger/hex/releases/download/${VERSION}/hex-${VERSION}-${TARGET}.tar.gz"
echo "Downloading hex ${VERSION} for ${TARGET}..."
curl -L "$URL" | tar xz -C /tmp
sudo mv /tmp/hex /usr/local/bin/hex
sudo mv /tmp/hex-nexus /usr/local/bin/hex-nexus
echo "hex ${VERSION} installed successfully"
hex --version
