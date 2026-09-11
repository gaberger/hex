#!/usr/bin/env bash
set -euo pipefail

# bump-patch.sh — Bump the patch version in Cargo.toml (the single source of truth).
# package.json was deleted with the phantom TypeScript library (wp-2608241500-hex-solo P0.4).
# Usage: ./scripts/bump-patch.sh [--dry-run]

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CARGO_TOML="$ROOT/Cargo.toml"

DRY_RUN=false
if [[ "${1:-}" == "--dry-run" ]]; then
  DRY_RUN=true
fi

# Extract current version from Cargo.toml (single source of truth)
CURRENT=$(grep '^version' "$CARGO_TOML" | head -1 | sed 's/.*"\([^"]*\)".*/\1/')

if [[ -z "$CURRENT" ]]; then
  echo "Error: could not read version from Cargo.toml" >&2
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

# Verify the file was updated
CARGO_VER=$(grep '^version' "$CARGO_TOML" | head -1 | sed 's/.*"\([^"]*\)".*/\1/')

if [[ "$CARGO_VER" != "$NEW_VERSION" ]]; then
  echo "Error: version mismatch after update" >&2
  echo "  Cargo.toml: $CARGO_VER" >&2
  exit 1
fi

echo "Updated Cargo.toml: $NEW_VERSION"
echo ""
echo "Next steps:"
echo "  git add Cargo.toml"
echo "  git commit -m \"chore: bump version to $NEW_VERSION\""
