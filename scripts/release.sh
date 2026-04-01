#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-}"

if [[ -z "$VERSION" ]]; then
  echo "Usage: $0 <version>"
  echo "  Example: $0 0.5.0"
  exit 1
fi

# Validate semver-ish format
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9._-]+)?(\+[a-zA-Z0-9._-]+)?$ ]]; then
  echo "Error: version must be semver format (e.g. 0.5.0)"
  exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "Bumping workspace version to $VERSION..."

# All crates use version.workspace = true — only update root Cargo.toml
sed -i.bak "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml
rm -f Cargo.toml.bak

echo "Running cargo check to update Cargo.lock..."
cargo check -p hex-cli -p hex-nexus 2>&1 | tail -5

echo "Staging changes..."
git add Cargo.toml Cargo.lock

echo "Creating release commit..."
git commit -m "chore(release): bump version to v${VERSION}"

echo "Creating tag v${VERSION}..."
git tag -a "v${VERSION}" -m "Release v${VERSION}"

echo ""
echo "Done. Push to trigger the release workflow:"
echo ""
echo "  git push origin main --tags"
echo ""
