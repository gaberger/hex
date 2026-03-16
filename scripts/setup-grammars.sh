#!/usr/bin/env bash
#
# Download pre-built tree-sitter WASM grammars to config/grammars/.
# Run this if 'bun add tree-sitter-wasms' is not an option.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
GRAMMAR_DIR="$PROJECT_DIR/config/grammars"

REPO="nicolo-ribaudo/tree-sitter-wasm-builds"
TAG="latest"
BASE_URL="https://github.com/$REPO/releases/download"

LANGUAGES=(typescript go rust)

mkdir -p "$GRAMMAR_DIR"

echo "Downloading tree-sitter WASM grammars to $GRAMMAR_DIR ..."

for lang in "${LANGUAGES[@]}"; do
  filename="tree-sitter-${lang}.wasm"
  target="$GRAMMAR_DIR/$filename"

  if [ -f "$target" ]; then
    echo "  [skip] $filename already exists"
    continue
  fi

  # Try npm package first (preferred)
  npm_path="$PROJECT_DIR/node_modules/tree-sitter-wasms/out/$filename"
  if [ -f "$npm_path" ]; then
    echo "  [copy] $filename from node_modules/tree-sitter-wasms"
    cp "$npm_path" "$target"
    continue
  fi

  # Fall back to GitHub release download
  url="$BASE_URL/$TAG/$filename"
  echo "  [download] $filename from $url"
  if command -v curl &>/dev/null; then
    curl -fsSL -o "$target" "$url" || echo "  [WARN] Failed to download $filename"
  elif command -v wget &>/dev/null; then
    wget -q -O "$target" "$url" || echo "  [WARN] Failed to download $filename"
  else
    echo "  [ERROR] Neither curl nor wget found. Cannot download $filename."
    exit 1
  fi
done

echo ""
echo "Done. Grammars installed in $GRAMMAR_DIR"
echo "You can now run: hex analyze ./src"
