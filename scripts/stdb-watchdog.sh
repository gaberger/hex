#!/usr/bin/env bash
# stdb-watchdog.sh — keep SpacetimeDB alive across upstream BsatnError panics.
#
# Pings http://127.0.0.1:3033/v1/ping every WATCH_INTERVAL seconds. If
# unreachable for two consecutive checks, kills any stale process,
# spawns a fresh spacetimedb-standalone, waits for ping, and republishes
# hexflo-coordination so the schema is fresh.
#
# Usage:
#   nohup scripts/stdb-watchdog.sh >> ~/.hex/stdb-watchdog.log 2>&1 &
#   disown
#
# Env overrides:
#   STDB_BIN, STDB_DATA, STDB_KEYS, STDB_LISTEN, WATCH_INTERVAL, WASM_PATH

set -uo pipefail

STDB_BIN=${STDB_BIN:-$HOME/.local/bin/spacetimedb-standalone}
STDB_DATA=${STDB_DATA:-$HOME/.local/share/spacetime/data}
STDB_KEYS=${STDB_KEYS:-$HOME/.config/spacetime/}
STDB_LISTEN=${STDB_LISTEN:-0.0.0.0:3033}
WATCH_INTERVAL=${WATCH_INTERVAL:-30}
WASM_PATH=${WASM_PATH:-$HOME/hex-intf/hex-cli/assets/wasm/hexflo_coordination.wasm}
SPACETIME_BIN=${SPACETIME_BIN:-$HOME/.local/bin/spacetime}

log() { echo "[$(date -u +%Y-%m-%dT%H:%M:%SZ)] $*"; }

start_stdb() {
  log "spawning spacetimedb-standalone"
  setsid "$STDB_BIN" start \
    --data-dir "$STDB_DATA" \
    --jwt-key-dir "$STDB_KEYS" \
    --listen-addr "$STDB_LISTEN" \
    >> "$HOME/.hex/spacetimedb.log" 2>&1 < /dev/null &
  disown
}

wait_for_ping() {
  for _ in $(seq 1 30); do
    if curl -sf http://127.0.0.1:3033/v1/ping >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  return 1
}

republish_module() {
  if [[ -x "$SPACETIME_BIN" && -f "$WASM_PATH" ]]; then
    log "republishing hexflo-coordination from $WASM_PATH"
    "$SPACETIME_BIN" publish -b "$WASM_PATH" --server local hex --yes >/dev/null 2>&1 \
      && log "republish OK" \
      || log "republish FAILED"
  else
    log "skipping republish (spacetime CLI or WASM missing)"
  fi
}

log "stdb-watchdog starting interval=${WATCH_INTERVAL}s"

dead_streak=0
while true; do
  if curl -sf http://127.0.0.1:3033/v1/ping >/dev/null 2>&1; then
    dead_streak=0
  else
    dead_streak=$((dead_streak + 1))
    log "ping FAILED (streak=$dead_streak)"
    if [[ $dead_streak -ge 2 ]]; then
      log "STDB confirmed dead — recovering"
      pkill -9 -f 'spacetimedb-standalone start' 2>/dev/null || true
      sleep 2
      start_stdb
      if wait_for_ping; then
        log "STDB ping recovered"
        republish_module
      else
        log "STDB ping NOT recovered after 60s — leaving for next cycle"
      fi
      dead_streak=0
    fi
  fi
  sleep "$WATCH_INTERVAL"
done
