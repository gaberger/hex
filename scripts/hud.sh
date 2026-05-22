#!/usr/bin/env bash
# hud.sh — live thermal/inference dashboard for Strix Halo dev hosts.
#
# Refreshes every $INTERVAL (default 2) seconds. Shows:
#   - CPU package + iGPU temperatures (lm-sensors)
#   - fan speeds
#   - iGPU busy %, power draw, VRAM use (amdgpu sysfs)
#   - currently-loaded Ollama models + VRAM each
#   - load average + system idle
#
# Usage:
#   bash scripts/hud.sh                # default 2s refresh
#   INTERVAL=5 bash scripts/hud.sh     # slower refresh
#
# This is a diagnostic utility, NOT runtime functionality (per
# CLAUDE.md HARD RULE). Use it to chase fan-noise / thermal puzzles
# during dev — the actual supervision/autopause loop lives in
# hex-nexus/src/orchestration/pool_autopause.rs.

set -u
INTERVAL="${INTERVAL:-2}"

# Pick the first amdgpu card we find. Strix Halo exposes it as card1
# (card0 is usually the display GPU virtual node).
GPU_DEV=""
for c in /sys/class/drm/card*/device; do
  if [ -f "$c/gpu_busy_percent" ]; then
    GPU_DEV="$c"
    break
  fi
done

# Locate the hwmon node that has power1_input. AMD APUs put it under
# /sys/class/drm/cardN/device/hwmon/hwmonM — number isn't stable.
GPU_PWR=""
if [ -n "$GPU_DEV" ]; then
  for h in "$GPU_DEV"/hwmon/hwmon*/power1_input; do
    if [ -r "$h" ]; then
      GPU_PWR="$h"
      break
    fi
  done
fi

bytes_to_gb() {
  awk '{ printf "%.2f", $1 / 1073741824 }'
}
microW_to_W() {
  awk '{ printf "%.1f", $1 / 1000000 }'
}

render_once() {
  printf '\033[H\033[2J' # clear screen + cursor home
  printf '== hex HUD ==  refresh=%ss   %s\n\n' "$INTERVAL" "$(date '+%Y-%m-%d %H:%M:%S')"

  printf 'TEMP / FAN\n'
  printf -- '----------\n'
  if command -v sensors >/dev/null 2>&1; then
    sensors 2>/dev/null | grep -E 'fan[0-9]:|temp[0-9]:' | head -10
  else
    printf '  (lm-sensors not installed)\n'
  fi
  printf '\n'

  printf 'iGPU\n'
  printf -- '----\n'
  if [ -n "$GPU_DEV" ]; then
    busy=$(cat "$GPU_DEV/gpu_busy_percent" 2>/dev/null || echo '?')
    pwr='?'
    [ -n "$GPU_PWR" ] && pwr="$(microW_to_W < "$GPU_PWR")W"
    vram_u='?' vram_t='?'
    if [ -r "$GPU_DEV/mem_info_vram_used" ]; then
      vram_u="$(bytes_to_gb < "$GPU_DEV/mem_info_vram_used")GB"
      vram_t="$(bytes_to_gb < "$GPU_DEV/mem_info_vram_total")GB"
    fi
    printf '  busy=%s%%   power=%s   vram=%s / %s\n' "$busy" "$pwr" "$vram_u" "$vram_t"
  else
    printf '  (no amdgpu sysfs surface found)\n'
  fi
  printf '\n'

  printf 'OLLAMA\n'
  printf -- '------\n'
  ollama_resp=$(curl -sS --max-time 3 http://localhost:11434/api/ps 2>/dev/null || echo '')
  if [ -n "$ollama_resp" ]; then
    echo "$ollama_resp" | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    print("  (parse failed)")
    sys.exit(0)
models = d.get("models", [])
if not models:
    print("  (no models loaded)")
for m in models:
    name = m.get("name", "?")
    mb = m.get("size_vram", 0) // 1024 // 1024
    expires = m.get("expires_at", "")
    print(f"  {name:34} {mb:>5} MB VRAM   expires={expires[:19]}")
' 2>/dev/null
  else
    printf '  (ollama unreachable on :11434)\n'
  fi
  printf '\n'

  printf 'SYSTEM\n'
  printf -- '------\n'
  printf '  load: %s\n' "$(awk '{print $1, $2, $3}' /proc/loadavg)"
  if command -v top >/dev/null 2>&1; then
    idle=$(top -bn1 | grep -m1 'Cpu(s)' | awk -F'[, ]+' '{ for(i=1;i<=NF;i++) if($i=="id") {print $(i-1); exit} }')
    printf '  cpu idle: %s%%\n' "$idle"
  fi
}

# If running interactively / has stdout-tty, render in a loop. Otherwise
# print one snapshot and exit (CI / piped use).
if [ -t 1 ]; then
  trap 'printf "\n"; exit 0' INT
  while true; do
    render_once
    sleep "$INTERVAL"
  done
else
  render_once
fi
