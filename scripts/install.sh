#!/bin/bash
# hex installer — fetches release binaries (hex, hex-nexus, hex-agent),
# installs SpacetimeDB if missing, generates JWT keys, and verifies the
# install end-to-end. Idempotent; safe to re-run.
#
# Usage:
#   ./install.sh                 # install latest
#   ./install.sh 26.5.1          # pin to a version
#   ./install.sh --check         # verify-only mode (no download/install)
#   INSTALL_PREFIX=/usr/local ./install.sh   # override target dir

set -euo pipefail

# ── Color helpers ─────────────────────────────────────────────────────
if [ -t 1 ]; then
  c_red()  { printf '\033[31m%s\033[0m\n' "$*"; }
  c_grn()  { printf '\033[32m%s\033[0m\n' "$*"; }
  c_ylw()  { printf '\033[33m%s\033[0m\n' "$*"; }
  c_bold() { printf '\033[1m%s\033[0m\n' "$*"; }
else
  c_red()  { printf '%s\n' "$*"; }
  c_grn()  { printf '%s\n' "$*"; }
  c_ylw()  { printf '%s\n' "$*"; }
  c_bold() { printf '%s\n' "$*"; }
fi
ok()   { c_grn "  ✓ $*"; }
warn() { c_ylw "  ⚠ $*"; }
fail() { c_red "  ✗ $*"; }

# ── Args ──────────────────────────────────────────────────────────────
CHECK_ONLY=false
WITH_OLLAMA=false
SKIP_CHECKSUM=false
VERSION=latest
for arg in "$@"; do
  case "$arg" in
    --check)          CHECK_ONLY=true ;;
    --with-ollama)    WITH_OLLAMA=true ;;
    --skip-checksum)  SKIP_CHECKSUM=true ;;
    --help|-h)
      cat <<'EOF'
hex installer

Usage:
  install.sh [version]         install latest (or pinned) release
  install.sh --check           verify existing install — no download
  install.sh --with-ollama     also install Ollama (for local inference)
  install.sh --skip-checksum   skip SHA256 verification (NOT recommended)
  install.sh --help            this message

Env:
  INSTALL_PREFIX=/usr/local       override install dir (default: ~/.local on linux, /usr/local on macos)
  HEX_INSTALL_SKIP_STDB=1         skip SpacetimeDB install (assume present)
EOF
      exit 0 ;;
    *) VERSION="$arg" ;;
  esac
done

# ── Platform detection ────────────────────────────────────────────────
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

if [ "$OS" = "darwin" ] && [ "$ARCH" = "arm64" ]; then
  TARGET="aarch64-apple-darwin"
elif [ "$OS" = "darwin" ] && [ "$ARCH" = "x86_64" ]; then
  TARGET="x86_64-apple-darwin"
elif [ "$OS" = "linux" ] && [ "$ARCH" = "x86_64" ]; then
  TARGET="x86_64-unknown-linux-gnu"
elif [ "$OS" = "linux" ] && [ "$ARCH" = "aarch64" ]; then
  TARGET="aarch64-unknown-linux-gnu"
else
  fail "Unsupported platform: $OS/$ARCH (supported: linux/x86_64, linux/aarch64, darwin/arm64, darwin/x86_64)"
  exit 1
fi

# Install prefix (immutable distros default to user-writable ~/.local/bin)
if [ -n "${INSTALL_PREFIX:-}" ]; then
  BIN_DIR="$INSTALL_PREFIX/bin"
elif [ "$OS" = "linux" ]; then
  BIN_DIR="${HOME}/.local/bin"
else
  BIN_DIR="/usr/local/bin"
fi

c_bold "hex installer"
echo "  target:     $TARGET"
echo "  install to: $BIN_DIR"
echo "  mode:       $([ "$CHECK_ONLY" = true ] && echo verify-only || echo "install (v$VERSION)")"
echo

# ── Verification helpers ──────────────────────────────────────────────
verify_binary_exists() {
  local name="$1" path="$BIN_DIR/$1"
  if [ -x "$path" ]; then
    ok "$name installed ($path)"
    return 0
  else
    fail "$name missing or not executable at $path"
    return 1
  fi
}

verify_binary_runs() {
  local name="$1" path="$BIN_DIR/$1" flag="${2:---version}"
  if "$path" "$flag" >/dev/null 2>&1; then
    local ver
    ver=$("$path" "$flag" 2>&1 | head -1)
    ok "$name runs: $ver"
    return 0
  else
    fail "$name installed but won't run with $flag"
    return 1
  fi
}

verify_on_path() {
  local name="$1"
  if command -v "$name" >/dev/null 2>&1; then
    local resolved
    resolved=$(command -v "$name")
    if [ "$resolved" = "$BIN_DIR/$name" ]; then
      ok "$name on PATH → $resolved"
    else
      warn "$name on PATH but resolves to $resolved (not $BIN_DIR/$name)"
    fi
    return 0
  else
    fail "$name not on PATH — add $BIN_DIR to PATH (e.g. add to ~/.bashrc: export PATH=\"$BIN_DIR:\$PATH\")"
    return 1
  fi
}

verify_jwt_keys() {
  local cfg="${HOME}/.config/spacetime"
  if [ -f "$cfg/id_ecdsa" ] && [ -f "$cfg/id_ecdsa.pub" ]; then
    local perms
    perms=$(stat -c '%a' "$cfg/id_ecdsa" 2>/dev/null || stat -f '%Lp' "$cfg/id_ecdsa" 2>/dev/null)
    if [ "$perms" = "600" ] || [ "$perms" = "400" ]; then
      ok "SpacetimeDB JWT keys present + private-key 0$perms ($cfg)"
    else
      warn "SpacetimeDB JWT keys present but private-key perms 0$perms (should be 0600 or 0400)"
    fi
    return 0
  else
    fail "SpacetimeDB JWT keys missing at $cfg/id_ecdsa{,.pub}"
    return 1
  fi
}

# ── Pre-check (always runs) ──────────────────────────────────────────
c_bold "Pre-flight checks"

for tool in curl tar; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    fail "$tool not found — required to download + extract release tarball"
    exit 1
  fi
done
ok "curl + tar present"

if ! command -v openssl >/dev/null 2>&1; then
  warn "openssl not found — required for SpacetimeDB JWT key generation"
  warn "  install: brew install openssl   OR   sudo apt install openssl"
fi

# Path-writable check
if [ ! -d "$BIN_DIR" ]; then
  if mkdir -p "$BIN_DIR" 2>/dev/null; then
    ok "created install dir $BIN_DIR"
  else
    fail "cannot create $BIN_DIR — try INSTALL_PREFIX=… or run with sudo"
    exit 1
  fi
fi

if [ ! -w "$BIN_DIR" ]; then
  warn "$BIN_DIR is not writable — install will use sudo for moves"
  HAVE_SUDO=true
else
  HAVE_SUDO=false
fi
echo

# ── Verify-only path ────────────────────────────────────────────────
if [ "$CHECK_ONLY" = true ]; then
  c_bold "Verifying existing install"
  failed=0
  # Required binaries — missing = real failure
  for bin in hex hex-nexus; do
    verify_binary_exists "$bin" || failed=$((failed+1))
    verify_binary_runs "$bin" --version >/dev/null 2>&1 || { fail "$bin won't run"; failed=$((failed+1)); }
    verify_on_path "$bin" || failed=$((failed+1))
  done
  # Optional binaries — missing = warn only (legacy tarballs)
  for bin in hex-agent; do
    if [ -x "$BIN_DIR/$bin" ]; then
      verify_binary_runs "$bin" --version >/dev/null 2>&1 && ok "$bin runs" || { fail "$bin won't run"; failed=$((failed+1)); }
      verify_on_path "$bin" >/dev/null 2>&1 && ok "$bin on PATH" || true
    else
      warn "$bin not installed (optional — present in releases after 2026-05-23)"
    fi
  done
  echo
  c_bold "SpacetimeDB"
  for bin in spacetime spacetimedb-standalone; do
    verify_binary_exists "$bin" || failed=$((failed+1))
  done
  verify_jwt_keys || failed=$((failed+1))
  echo
  if [ "$failed" -eq 0 ]; then
    c_grn "✓ Install verified — $("$BIN_DIR/hex" --version 2>/dev/null | head -1)"
    exit 0
  else
    c_red "✗ $failed check(s) failed — run install.sh to repair"
    exit 1
  fi
fi

# ── Download + install ───────────────────────────────────────────────
c_bold "Downloading hex release"

if [ "$VERSION" = "latest" ]; then
  VERSION=$(curl -sSf https://api.github.com/repos/gaberger/hex/releases/latest \
            | grep '"tag_name"' | head -1 | cut -d'"' -f4)
  VERSION="${VERSION#v}"
  if [ -z "$VERSION" ]; then
    fail "could not resolve latest hex version from GitHub API"
    exit 1
  fi
  ok "resolved latest: $VERSION"
fi

URL="https://github.com/gaberger/hex/releases/download/v${VERSION}/hex-${VERSION}-${TARGET}.tar.gz"
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

if ! curl -fSL "$URL" -o "$TMPDIR/hex.tar.gz" 2>/dev/null; then
  fail "download failed: $URL"
  echo "  → check that release v$VERSION exists and tarball name matches target $TARGET"
  exit 1
fi
ok "downloaded hex v$VERSION"

# Checksum verification — fetch the release's SHA256SUMS.txt and verify
# our tarball's hash matches. Catches MITM, corrupted download, and the
# rare case where GitHub serves a stale CDN copy. Skip with --skip-checksum
# only for offline / GitHub-down debugging.
if [ "$SKIP_CHECKSUM" = true ]; then
  warn "skipping SHA256 verification (--skip-checksum)"
else
  SUMS_URL="https://github.com/gaberger/hex/releases/download/v${VERSION}/SHA256SUMS.txt"
  if ! curl -fSL "$SUMS_URL" -o "$TMPDIR/SHA256SUMS.txt" 2>/dev/null; then
    warn "SHA256SUMS.txt missing from release v$VERSION — skipping verification"
    warn "  → file releases prior to checksum-rollout don't have this; safe but less verified"
  else
    # SHA256SUMS.txt lines look like: "<sha256>  hex-<ver>-<target>.tar.gz"
    # Find our tarball's entry, extract the expected hash.
    tarball_name="hex-${VERSION}-${TARGET}.tar.gz"
    expected=$(grep "  $tarball_name\$" "$TMPDIR/SHA256SUMS.txt" | awk '{print $1}')
    if [ -z "$expected" ]; then
      fail "no SHA256 entry for $tarball_name in SHA256SUMS.txt"
      exit 1
    fi
    if command -v sha256sum >/dev/null 2>&1; then
      actual=$(sha256sum "$TMPDIR/hex.tar.gz" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
      actual=$(shasum -a 256 "$TMPDIR/hex.tar.gz" | awk '{print $1}')
    else
      warn "neither sha256sum nor shasum found — cannot verify checksum"
      actual=""
    fi
    if [ -n "$actual" ]; then
      if [ "$actual" = "$expected" ]; then
        ok "SHA256 verified ($expected)"
      else
        fail "SHA256 MISMATCH — refusing to install"
        echo "  expected: $expected"
        echo "  actual:   $actual"
        echo "  → this could indicate corrupted download or supply-chain tampering"
        exit 1
      fi
    fi
  fi
fi

if ! tar xz -C "$TMPDIR" -f "$TMPDIR/hex.tar.gz"; then
  fail "tarball extraction failed"
  exit 1
fi
ok "extracted"

# Inventory the tarball. hex + hex-nexus are required; hex-agent has
# been in transition — release workflow patched 2026-05-23 to bundle
# it, but tarballs published before that date don't have it. Treat as
# warn-not-fail so legacy releases still install, while flagging the
# gap so the user knows to upgrade to a fresher release.
required_binaries=(hex hex-nexus)
optional_binaries=(hex-agent)
for bin in "${required_binaries[@]}"; do
  if [ ! -f "$TMPDIR/$bin" ]; then
    fail "tarball missing required binary $bin — release v$VERSION is broken"
    exit 1
  fi
done
ok "tarball contains required binaries: ${required_binaries[*]}"
have_hex_agent=true
for bin in "${optional_binaries[@]}"; do
  if [ ! -f "$TMPDIR/$bin" ]; then
    warn "tarball missing optional $bin — release v$VERSION pre-dates the hex-agent rollout"
    warn "  → install proceeds without it; upgrade to the next release to get $bin"
    have_hex_agent=false
  fi
done
[ "$have_hex_agent" = true ] && ok "tarball contains hex-agent"

# Stop running hex processes before replacing binaries
if pgrep -x "hex-nexus" >/dev/null 2>&1 || pgrep -x "hex-agent" >/dev/null 2>&1; then
  warn "stopping running hex processes"
  killall hex-nexus hex-agent 2>/dev/null || true
  sleep 2
  pkill -9 -f "hex dev|hex agent worker|hex-nexus|hex-agent" 2>/dev/null || true
  sleep 1
  ok "stopped"
fi

# Install each binary, preserving prior version as .prev for rollback
mv_cmd() {
  if [ "$HAVE_SUDO" = true ]; then
    sudo mv "$@"
  else
    mv "$@"
  fi
}

c_bold "Installing binaries to $BIN_DIR"
# Install required, then optional-if-present
for bin in "${required_binaries[@]}" "${optional_binaries[@]}"; do
  if [ ! -f "$TMPDIR/$bin" ]; then
    continue  # optional binary missing from this tarball — already warned
  fi
  if [ -f "$BIN_DIR/$bin" ]; then
    mv_cmd "$BIN_DIR/$bin" "$BIN_DIR/$bin.prev" 2>/dev/null || true
  fi
  mv_cmd "$TMPDIR/$bin" "$BIN_DIR/$bin"
  chmod +x "$BIN_DIR/$bin"
  ok "installed $bin"
done
echo

# ── SpacetimeDB ──────────────────────────────────────────────────────
if [ "${HEX_INSTALL_SKIP_STDB:-}" = "1" ]; then
  warn "skipping SpacetimeDB install (HEX_INSTALL_SKIP_STDB=1)"
else
  c_bold "SpacetimeDB"

  if command -v spacetimedb-standalone >/dev/null 2>&1 && command -v spacetime >/dev/null 2>&1; then
    ok "spacetime + spacetimedb-standalone already installed"
  else
    STDB_VERSION=$(curl -sSf https://api.github.com/repos/clockworklabs/SpacetimeDB/releases/latest \
                   | grep '"tag_name"' | head -1 | cut -d'"' -f4)
    if [ -z "$STDB_VERSION" ]; then
      fail "could not resolve latest SpacetimeDB version"
      echo "  → install manually: https://spacetimedb.com/install"
      exit 1
    fi
    ok "resolved SpacetimeDB version: $STDB_VERSION"

    STDB_URL="https://github.com/clockworklabs/SpacetimeDB/releases/download/${STDB_VERSION}/spacetime-${TARGET}.tar.gz"
    STDB_TMP=$(mktemp -d)
    if ! curl -fSL "$STDB_URL" -o "$STDB_TMP/spacetime.tar.gz" 2>/dev/null; then
      fail "SpacetimeDB download failed: $STDB_URL"
      rm -rf "$STDB_TMP"
      exit 1
    fi
    tar xz -C "$STDB_TMP" -f "$STDB_TMP/spacetime.tar.gz"

    for bin in spacetimedb-cli spacetimedb-standalone; do
      if [ ! -f "$STDB_TMP/$bin" ]; then
        fail "SpacetimeDB tarball missing $bin"
        rm -rf "$STDB_TMP"
        exit 1
      fi
    done

    mv_cmd "$STDB_TMP/spacetimedb-cli" "$BIN_DIR/spacetime"
    mv_cmd "$STDB_TMP/spacetimedb-standalone" "$BIN_DIR/spacetimedb-standalone"
    chmod +x "$BIN_DIR/spacetime" "$BIN_DIR/spacetimedb-standalone"
    rm -rf "$STDB_TMP"
    ok "installed spacetime + spacetimedb-standalone"
  fi

  # JWT key generation (required for spacetimedb-standalone to start)
  STDB_CONFIG="${HOME}/.config/spacetime"
  if [ ! -f "$STDB_CONFIG/id_ecdsa" ]; then
    if ! command -v openssl >/dev/null 2>&1; then
      fail "openssl required for JWT key generation — install openssl then re-run"
      exit 1
    fi
    mkdir -p "$STDB_CONFIG"
    openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:prime256v1 \
      -out "$STDB_CONFIG/id_ecdsa" 2>/dev/null
    openssl pkey -in "$STDB_CONFIG/id_ecdsa" -pubout \
      -out "$STDB_CONFIG/id_ecdsa.pub" 2>/dev/null
    chmod 600 "$STDB_CONFIG/id_ecdsa"
    chmod 644 "$STDB_CONFIG/id_ecdsa.pub"
    ok "generated SpacetimeDB JWT keys"
  else
    ok "SpacetimeDB JWT keys already exist"
  fi
fi
echo

# ── Ollama (opt-in via --with-ollama) ────────────────────────────────
# hex's local-first inference path uses Ollama. Without it, the system
# falls back to paid frontier models (Anthropic / OpenRouter) — fine
# for casual use, expensive for the always-on loops (workplan_auto_emitter,
# hive_improver, gap_dispatcher). Recommended for daily-driver use.
if [ "$WITH_OLLAMA" = true ]; then
  c_bold "Ollama"
  if command -v ollama >/dev/null 2>&1; then
    ok "ollama already installed: $(ollama --version 2>&1 | head -1)"
  else
    if [ "$OS" = "darwin" ]; then
      if command -v brew >/dev/null 2>&1; then
        warn "installing ollama via brew (interactive)"
        brew install ollama || { fail "brew install ollama failed"; exit 1; }
      else
        fail "macos ollama install requires brew — install brew first OR download from https://ollama.com/download"
        exit 1
      fi
    else
      warn "installing ollama via official installer (curl https://ollama.com/install.sh)"
      curl -fsSL https://ollama.com/install.sh | sh || { fail "ollama installer failed"; exit 1; }
    fi
    ok "ollama installed"
  fi
  # Don't auto-pull models — that's many GB and the user may want to
  # pick which ones. Just print the recommended set for dev mode.
  echo "  ℹ recommended models for local-first dev:"
  echo "      ollama pull qwen2.5-coder:14b   # T2 codegen (8 GB)"
  echo "      ollama pull nemotron-mini       # cheap summariser (2 GB)"
  echo "      ollama pull devstral-small-2:24b  # T2.5 reasoning (14 GB; optional)"
  echo
fi

# ── Post-install verification ────────────────────────────────────────
c_bold "Post-install verification"
verify_ok=true
# Required
for bin in "${required_binaries[@]}"; do
  verify_binary_runs "$bin" --version || verify_ok=false
  verify_on_path "$bin" || verify_ok=false
done
# Optional — only verify if it was installed (legacy tarballs may not include it)
for bin in "${optional_binaries[@]}"; do
  if [ -x "$BIN_DIR/$bin" ]; then
    verify_binary_runs "$bin" --version || verify_ok=false
    verify_on_path "$bin" || verify_ok=false
  fi
done

if [ "${HEX_INSTALL_SKIP_STDB:-}" != "1" ]; then
  verify_binary_exists spacetimedb-standalone || verify_ok=false
  verify_binary_exists spacetime || verify_ok=false
  verify_jwt_keys || verify_ok=false
fi

# Ollama is opt-in; report status informationally either way.
if command -v ollama >/dev/null 2>&1; then
  ok "ollama detected ($(command -v ollama)) — local-first inference enabled"
else
  warn "ollama NOT installed — install with: $0 --with-ollama  (local-first inference)"
  warn "  → without ollama, hex falls back to paid frontier models (ANTHROPIC_API_KEY / OPENROUTER_API_KEY)"
fi
echo

if [ "$verify_ok" = true ]; then
  c_grn "✓ Install complete — hex $(hex --version 2>/dev/null | head -1 | sed 's/^hex //')"
  echo
  echo "Next steps:"
  echo "  1. cd into your project   cd /path/to/your-project"
  echo "  2. bootstrap services     hex bootstrap"
  echo "  3. initialise hex         hex init"
  echo "  4. verify everything      hex doctor"
  echo
  echo "Or for a one-liner that does steps 2-4:"
  echo "  hex bootstrap && hex doctor"
else
  c_red "✗ Install completed but verification failed — re-run install.sh --check for details"
  exit 1
fi
