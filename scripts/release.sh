#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-}"

# Derive default version from current date in YY.MM.0 format
CURRENT_YY=$(date +%y)
CURRENT_MM=$(date +%-m)   # no leading zero — Cargo semver forbids it

if [[ -z "$VERSION" ]]; then
  echo "Usage: $0 <version>"
  echo "  Format: YY.MM.minor  (e.g. $(date +%y).$(date +%-m).0)"
  echo "  Minor increments within a month; YY.MM resets on month rollover."
  exit 1
fi

# Enforce YY.MM.minor — middle segment must match current month (no leading zero)
if ! [[ "$VERSION" =~ ^([0-9]{2})\.([0-9]+)\.([0-9]+)$ ]]; then
  echo "Error: version must be YY.MM.minor format (e.g. ${CURRENT_YY}.${CURRENT_MM}.0)"
  exit 1
fi

VERSION_YY="${BASH_REMATCH[1]}"
VERSION_MM="${BASH_REMATCH[2]}"

if [[ "$VERSION_YY" != "$CURRENT_YY" || "$VERSION_MM" != "$CURRENT_MM" ]]; then
  echo "Error: version YY.MM must match today (${CURRENT_YY}.${CURRENT_MM}), got ${VERSION_YY}.${VERSION_MM}"
  echo "  Did you mean ${CURRENT_YY}.${CURRENT_MM}.${BASH_REMATCH[3]}?"
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
