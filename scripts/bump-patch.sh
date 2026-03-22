#!/usr/bin/env bash
set -euo pipefail

# bump-patch.sh — Atomically bump the patch version in Cargo.toml and package.json
# Usage: ./scripts/bump-patch.sh [--dry-run]

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CARGO_TOML="$ROOT/Cargo.toml"
PACKAGE_JSON="$ROOT/package.json"

DRY_RUN=false
if [[ "${1:-}" == "--dry-run" ]]; then
  DRY_RUN=true
fi

# Extract current version from package.json (single source of truth)
CURRENT=$(grep '"version"' "$PACKAGE_JSON" | head -1 | sed 's/.*"version": *"\([^"]*\)".*/\1/')

if [[ -z "$CURRENT" ]]; then
  echo "Error: could not read version from package.json" >&2
  exit 1
fi

# Parse semver components
IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT"
NEW_PATCH=$((PATCH + 1))
NEW_VERSION="${MAJOR}.${MINOR}.${NEW_PATCH}"

echo "Bumping version: $CURRENT → $NEW_VERSION"

if $DRY_RUN; then
  echo "(dry run — no files modified)"
  exit 0
fi

# Update Cargo.toml workspace version
sed -i.bak "s/^version = \"$CURRENT\"/version = \"$NEW_VERSION\"/" "$CARGO_TOML"
rm -f "$CARGO_TOML.bak"

# Update package.json version
sed -i.bak "s/\"version\": \"$CURRENT\"/\"version\": \"$NEW_VERSION\"/" "$PACKAGE_JSON"
rm -f "$PACKAGE_JSON.bak"

# Verify both files were updated
CARGO_VER=$(grep '^version' "$CARGO_TOML" | head -1 | sed 's/.*"\([^"]*\)".*/\1/')
PKG_VER=$(grep '"version"' "$PACKAGE_JSON" | head -1 | sed 's/.*"\([^"]*\)".*/\1/')

if [[ "$CARGO_VER" != "$NEW_VERSION" || "$PKG_VER" != "$NEW_VERSION" ]]; then
  echo "Error: version mismatch after update" >&2
  echo "  Cargo.toml: $CARGO_VER" >&2
  echo "  package.json: $PKG_VER" >&2
  exit 1
fi

echo "Updated Cargo.toml:  $NEW_VERSION"
echo "Updated package.json: $NEW_VERSION"
echo ""
echo "Next steps:"
echo "  git add Cargo.toml package.json"
echo "  git commit -m \"chore: bump version to $NEW_VERSION\""
