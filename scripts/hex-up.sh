#!/usr/bin/env bash
# hex-up — bring up the whole hex stack idempotently.
#
# Usage:
#   ./scripts/hex-up.sh                # start STDB + nexus + sched
#   ./scripts/hex-up.sh --no-sched     # skip the sched daemon (foundation work mode)
#   ./scripts/hex-up.sh --rebuild      # force cargo rebuild before deploy
#   ./scripts/hex-up.sh --status       # just print the status, don't start anything
#
# Idempotent: safe to run multiple times. Existing daemons are left alone
# unless --restart is passed.

set -euo pipefail

REPO=${HEX_REPO:-/home/gary/hex-intf}
NEXUS_BIN=${HEX_NEXUS_BIN:-$HOME/.local/bin/hex-nexus}
HEX_BIN=${HEX_BIN:-$HOME/.local/bin/hex}
STDB_BIN=${HEX_STDB_BIN:-$HOME/.local/bin/spacetimedb-standalone}
LOG_DIR=${HEX_LOG_DIR:-$HOME/.hex}
mkdir -p "$LOG_DIR"

# Enable the typed-tool SOP path for executive personas (ADR-2026-05-08-2500).
# Without this, is_sop_persona() returns false and personas fall back to
# the OLD commitment-creator contract (no code_patch emission, just
# "Confirm: I will fix..." text replies). Set BEFORE starting nexus so
# the daemon process inherits it.
export HEX_SOP_PERSONAS=${HEX_SOP_PERSONAS:-cto,cpo,coo,ciso,chief-visionary,chief-architect}

# Ensure cargo + rustup binaries are reachable for the cargo_check tool
# spawn. nexus runtime PATH otherwise excludes ~/.cargo/bin.
case ":$PATH:" in
  *":$HOME/.cargo/bin:"*) ;;
  *) export PATH="$HOME/.cargo/bin:$PATH" ;;
esac

NO_SCHED=0
REBUILD=0
RESTART=0
STATUS_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --no-sched) NO_SCHED=1 ;;
    --rebuild)  REBUILD=1 ;;
    --restart)  RESTART=1 ;;
    --status)   STATUS_ONLY=1 ;;
    -h|--help)
      grep '^# ' "$0" | sed 's/^# \?//'
      exit 0 ;;
  esac
done

green() { printf '\e[32m%s\e[0m\n' "$1"; }
red()   { printf '\e[31m%s\e[0m\n' "$1"; }
yellow(){ printf '\e[33m%s\e[0m\n' "$1"; }

# ── status ────────────────────────────────────────────────
status() {
  echo
  echo "── HEX STACK STATUS ──"
  if pgrep -af 'spacetimedb-standalone start' >/dev/null; then
    green   "  spacetimedb : up"
  else
    red     "  spacetimedb : DOWN"
  fi
  if pgrep -af "$NEXUS_BIN --port" >/dev/null; then
    green   "  hex-nexus   : up"
  else
    red     "  hex-nexus   : DOWN"
  fi
  if pgrep -af 'hex sched daemon' >/dev/null; then
    green   "  sched daemon: up"
  else
    yellow  "  sched daemon: down"
  fi
  echo
  if curl -sS -m 1 http://127.0.0.1:5555/api/version >/dev/null 2>&1; then
    green   "  nexus API   : http://127.0.0.1:5555 (reachable)"
  else
    red     "  nexus API   : not reachable"
  fi
  if curl -sS -m 1 -X POST http://127.0.0.1:3033/v1/database/hex/sql -H 'Content-Type: text/plain' -d 'SELECT role FROM persona_pool LIMIT 1' 2>/dev/null | grep -v 'no such table' > /dev/null; then
    green   "  STDB hex db : reachable, persona_pool present"
  else
    red     "  STDB hex db : missing persona_pool (run persona_init?)"
  fi
  echo
  if [ -d "$REPO/docs/workplans" ]; then
    n_active=$(ls "$REPO/docs/workplans"/wp-*.json 2>/dev/null | wc -l)
    n_archived=$(ls "$REPO/docs/workplans/archive/done-2026-05-08/" 2>/dev/null | wc -l)
    echo "  workplans   : $n_active active, $n_archived archived"
  fi
}

if [ "$STATUS_ONLY" = 1 ]; then
  status
  exit 0
fi

# ── rebuild ───────────────────────────────────────────────
if [ "$REBUILD" = 1 ]; then
  echo "── Rebuilding nexus + cli ──"
  cd "$REPO"
  HEX_HUB_BUILD_HASH=$(git rev-parse HEAD | cut -c1-12) \
    PATH="$HOME/.cargo/bin:$PATH" \
    cargo build --release --bin hex-nexus --bin hex
  cp -v "$REPO/target/x86_64-unknown-linux-gnu/release/hex-nexus" "$NEXUS_BIN.new" 2>/dev/null \
    || cp -v "$REPO/target/release/hex-nexus" "$NEXUS_BIN.new"
  mv "$NEXUS_BIN.new" "$NEXUS_BIN"
  cp -v "$REPO/target/x86_64-unknown-linux-gnu/release/hex" "$HEX_BIN.new" 2>/dev/null \
    || cp -v "$REPO/target/release/hex" "$HEX_BIN.new"
  mv "$HEX_BIN.new" "$HEX_BIN"
  green "  binaries deployed"
fi

# ── stdb ──────────────────────────────────────────────────
echo "── SpacetimeDB ──"
if pgrep -af 'spacetimedb-standalone start' >/dev/null && [ "$RESTART" = 0 ]; then
  green "  already running"
else
  pkill -f 'spacetimedb-standalone start' 2>/dev/null || true
  sleep 2
  nohup "$STDB_BIN" start \
    --data-dir "$HOME/.local/share/spacetime/data" \
    --jwt-key-dir "$HOME/.config/spacetime/" \
    --listen-addr 0.0.0.0:3033 \
    >> "$LOG_DIR/spacetimedb.log" 2>&1 &
  disown
  green "  started"
  # Wait for ping
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sS -m 1 http://127.0.0.1:3033/v1/ping >/dev/null 2>&1; then break; fi
    sleep 2
  done
fi

# ── nexus ─────────────────────────────────────────────────
echo "── hex-nexus ──"
if pgrep -af "$NEXUS_BIN --port" >/dev/null && [ "$RESTART" = 0 ]; then
  green "  already running"
else
  pkill -f "$NEXUS_BIN --port" 2>/dev/null || true
  sleep 3
  cd "$REPO" || exit 1
  nohup "$NEXUS_BIN" --port 5555 --bind 0.0.0.0 --daemon \
    >> "$LOG_DIR/nexus.log" 2>&1 &
  disown
  for _ in 1 2 3 4 5 6 7 8 9 10 11 12; do
    if curl -sS -m 1 http://127.0.0.1:5555/api/version >/dev/null 2>&1; then break; fi
    sleep 2
  done
  green "  started (auto-init will seed personas + merge_quorum_policy in ~10s)"
fi

# ── sched daemon (optional) ──────────────────────────────
if [ "$NO_SCHED" = 0 ]; then
  echo "── hex sched daemon ──"
  if pgrep -af 'hex sched daemon' >/dev/null && [ "$RESTART" = 0 ]; then
    green "  already running"
  else
    pkill -f 'hex sched daemon' 2>/dev/null || true
    sleep 1
    nohup "$HEX_BIN" sched daemon --interval 30 --max-failures 3 \
      >> "$LOG_DIR/sched-daemon.log" 2>&1 &
    disown
    green "  started"
  fi
fi

sleep 5
status

echo
echo "── Quick verification ──"
echo "  hex worktree status     # see merge gate state"
echo "  curl http://127.0.0.1:5555/api/hex-agents | jq '.agents | length'  # should be 8 personas"
echo
echo "  Logs:"
echo "    $LOG_DIR/spacetimedb.log"
echo "    $LOG_DIR/nexus.log"
echo "    $LOG_DIR/sched-daemon.log"
