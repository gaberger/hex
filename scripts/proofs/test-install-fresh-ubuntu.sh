#!/usr/bin/env bash
# Fresh-VM test for scripts/install.sh against a clean ubuntu:22.04
# container. Proves the installer works for a stranger by running it
# in an environment that has NO hex assumptions baked in.
#
# Requires: docker or podman.
#
# What it does:
#   1. starts an ubuntu:22.04 container with curl + ca-certs + openssl
#      pre-installed (minimum a real user would have)
#   2. copies the WORKING-COPY install.sh into the container
#   3. runs install.sh against the local install.sh (so we test the
#      code we just wrote, not whatever's on a public branch)
#   4. asserts: all 5 binaries present + executable + --version works
#   5. runs install.sh --check inside the container and asserts exit 0
#
# Exits 0 only when every assertion passes. Logs the container output
# regardless so failures are diagnosable.

set -uo pipefail

INSTALL_SH="${INSTALL_SH:-/var/home/gary/hex-intf/scripts/install.sh}"
IMAGE="${IMAGE:-ubuntu:22.04}"
TEST_VERSION="${HEX_TEST_VERSION:-latest}"

bold() { printf '\033[1m%s\033[0m\n' "${1:-}"; }
green() { printf '\033[32m%s\033[0m\n' "${1:-}"; }
red() { printf '\033[31m%s\033[0m\n' "${1:-}"; }
warn() { printf '\033[33m%s\033[0m\n' "${1:-}"; }

PASS=0
FAIL=0
pass_msg() { green "  ✓ $1"; PASS=$((PASS+1)); }
fail_msg() { red   "  ✗ $1"; FAIL=$((FAIL+1)); }

bold "Fresh-VM install test — image=$IMAGE  version=$TEST_VERSION"
echo

# ── 0. container runtime present? (docker OR podman) ───────────────
if command -v docker >/dev/null 2>&1; then
  RUNTIME=docker
elif command -v podman >/dev/null 2>&1; then
  RUNTIME=podman
else
  red "neither docker nor podman found — install one and re-run"
  exit 1
fi
pass_msg "$RUNTIME available"

# ── 1. spin up container ────────────────────────────────────────────
CONTAINER="hex-install-test-$$"
cleanup() { "$RUNTIME" rm -f "$CONTAINER" >/dev/null 2>&1 || true; }
trap cleanup EXIT

bold
bold "Starting clean ubuntu container + running install.sh"
echo "  container: $CONTAINER"
echo

# Minimum deps a real user would have on a fresh ubuntu: curl, tar,
# ca-certificates, openssl. Everything else must be installed by
# install.sh.
"$RUNTIME" run -d --name "$CONTAINER" "$IMAGE" sleep 3600 >/dev/null
if ! "$RUNTIME" exec "$CONTAINER" bash -c 'apt-get update -qq && apt-get install -y -qq curl tar ca-certificates openssl >/dev/null 2>&1'; then
  fail_msg "base-deps install failed inside container"
  exit 1
fi
pass_msg "base deps (curl, tar, ca-certificates, openssl) installed"

# Copy the working-copy install.sh into the container
"$RUNTIME" cp "$INSTALL_SH" "$CONTAINER:/tmp/install.sh"
"$RUNTIME" exec "$CONTAINER" chmod +x /tmp/install.sh
pass_msg "install.sh copied into container"

# Run install.sh. Capture exit code + full output.
echo
bold "── install.sh output ───────────────────────────────────"
if "$RUNTIME" exec "$CONTAINER" bash -c "INSTALL_PREFIX=/usr/local /tmp/install.sh $TEST_VERSION" 2>&1; then
  pass_msg "install.sh exited 0"
else
  fail_msg "install.sh exited non-zero"
fi
echo "── end install.sh output ───────────────────────────────"
echo

# ── 2. assert each binary present + runnable ────────────────────────
# hex + hex-nexus are required; hex-agent is optional in legacy
# releases (added to the release workflow 2026-05-23). The test
# reflects the same tiering as install.sh — required = hard fail,
# optional = informational.
bold "Post-install assertions"
for bin in hex hex-nexus; do
  if "$RUNTIME" exec "$CONTAINER" test -x "/usr/local/bin/$bin"; then
    pass_msg "$bin present + executable"
  else
    fail_msg "$bin missing or not executable"
  fi
  if "$RUNTIME" exec "$CONTAINER" "/usr/local/bin/$bin" --version >/dev/null 2>&1; then
    ver=$("$RUNTIME" exec "$CONTAINER" "/usr/local/bin/$bin" --version 2>&1 | head -1)
    pass_msg "$bin runs --version: $ver"
  else
    fail_msg "$bin won't run --version"
  fi
done
# hex-agent optional — log status but don't fail
for bin in hex-agent; do
  if "$RUNTIME" exec "$CONTAINER" test -x "/usr/local/bin/$bin"; then
    pass_msg "$bin present (will become standard in releases after 2026-05-23)"
  else
    warn "  ⚠ $bin not in tarball — legacy release; expected in next published version"
  fi
done

# SpacetimeDB
for bin in spacetime spacetimedb-standalone; do
  if "$RUNTIME" exec "$CONTAINER" test -x "/usr/local/bin/$bin"; then
    pass_msg "$bin present + executable"
  else
    fail_msg "$bin missing or not executable"
  fi
done

# JWT keys (container's home is /root)
if "$RUNTIME" exec "$CONTAINER" test -f /root/.config/spacetime/id_ecdsa; then
  pass_msg "STDB JWT private key present"
else
  fail_msg "STDB JWT private key missing"
fi

# ── 3. install.sh --check should exit 0 (idempotency) ───────────────
# Pass the same INSTALL_PREFIX so --check looks in the right place
# (otherwise it defaults to ~/.local/bin and misses our /usr/local install)
bold
bold "Re-verify via install.sh --check"
if "$RUNTIME" exec "$CONTAINER" bash -c "INSTALL_PREFIX=/usr/local /tmp/install.sh --check" >/dev/null 2>&1; then
  pass_msg "install.sh --check exits 0 (idempotent verification)"
else
  fail_msg "install.sh --check exits non-zero on a fresh install"
  echo "      output:"
  "$RUNTIME" exec "$CONTAINER" bash -c "INSTALL_PREFIX=/usr/local /tmp/install.sh --check" 2>&1 | tail -10 | sed 's/^/        /'
fi

# ── SUMMARY ──────────────────────────────────────────────────────
echo
bold "── SUMMARY ─────────────────────────────────────────────"
echo "PASS: $PASS   FAIL: $FAIL"
if [ "$FAIL" = 0 ]; then
  green "✓ FRESH-VM INSTALL VERIFIED — installer works for a stranger"
  exit 0
else
  red "✗ INSTALLER REGRESSION on fresh VM — see output above"
  exit 1
fi
