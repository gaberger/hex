#!/usr/bin/env bash
# hud-tui.sh — Charm-decorated thermal/inference dashboard.
#
# Reads /sys/class/hwmon + /proc directly (no lm-sensors required).
# Uses `gum` for bordered panels; falls back to scripts/hud.sh if absent.
#
# Usage:
#   bash scripts/hud-tui.sh
#   INTERVAL=5 bash scripts/hud-tui.sh
#   NO_COLOR=1 bash scripts/hud-tui.sh   # strip ANSI color
#
# Diagnostic utility only (per CLAUDE.md HARD RULE — supervision logic
# lives in hex-nexus, not in shell).

set -u
INTERVAL="${INTERVAL:-2}"

if ! command -v gum >/dev/null 2>&1; then
  echo "gum not found — falling back to scripts/hud.sh" >&2
  exec bash "$(dirname "$0")/hud.sh"
fi

# --- discover amdgpu sysfs nodes (used by iGPU panel) -----------------------
GPU_DEV=""
for c in /sys/class/drm/card*/device; do
  [ -f "$c/gpu_busy_percent" ] && { GPU_DEV="$c"; break; }
done
GPU_PWR=""
GPU_PWR_LABEL="power"
if [ -n "$GPU_DEV" ]; then
  for h in "$GPU_DEV"/hwmon/hwmon*/power1_input; do
    [ -r "$h" ] && { GPU_PWR="$h"; break; }
  done
  if [ -n "$GPU_PWR" ]; then
    lbl_file="${GPU_PWR%power1_input}power1_label"
    if [ -r "$lbl_file" ]; then
      raw=$(cat "$lbl_file" 2>/dev/null)
      case "$raw" in PPT) GPU_PWR_LABEL='APU·PPT' ;; *) GPU_PWR_LABEL="$raw" ;; esac
    fi
  fi
fi

# --- color + bar helpers ----------------------------------------------------
C_RESET=$'\033[0m'
if [ -n "${NO_COLOR:-}" ]; then
  c_green="" c_yellow="" c_red="" c_dim="" c_cyan="" c_bold="" c_blue="" c_mag=""
else
  c_green=$'\033[38;5;42m'
  c_yellow=$'\033[38;5;220m'
  c_red=$'\033[38;5;203m'
  c_dim=$'\033[38;5;244m'
  c_cyan=$'\033[38;5;87m'
  c_blue=$'\033[38;5;75m'
  c_mag=$'\033[38;5;177m'
  c_bold=$'\033[1m'
fi

# pick_color VALUE GOOD_MAX WARN_MAX  → outputs an ANSI color escape
pick_color() {
  awk -v v="$1" -v g="$2" -v w="$3" \
      -v G="$c_green" -v Y="$c_yellow" -v R="$c_red" -v D="$c_dim" '
    BEGIN {
      if (v == "" || v == "?") { print D; exit }
      if (v+0 <= g) print G
      else if (v+0 <= w) print Y
      else print R
    }'
}

# bar VALUE MAX WIDTH
bar() {
  local v="$1" max="$2" w="${3:-10}"
  awk -v v="$v" -v m="$max" -v w="$w" \
      -v G="$c_green" -v Y="$c_yellow" -v R="$c_red" -v D="$c_dim" -v X="$C_RESET" '
    BEGIN {
      if (v == "" || v == "?" || m+0 == 0) {
        for (i=0;i<w;i++) printf "%s", "·"
        exit
      }
      pct = (v+0) / (m+0)
      if (pct > 1) pct = 1
      filled = int(pct * w + 0.5)
      col = (pct < 0.5) ? G : (pct < 0.8) ? Y : R
      printf "%s", col
      for (i=0;i<filled;i++) printf "█"
      printf "%s", D
      for (i=filled;i<w;i++) printf "░"
      printf "%s", X
    }'
}

bytes_to_gb() { awk '{ printf "%.1f", $1 / 1073741824 }'; }
kb_to_gb()    { awk '{ printf "%.1f", $1 / 1048576 }'; }
microw_to_w() { awk '{ printf "%.1f", $1 / 1000000 }'; }

# read a single sysfs hwmon temp by (name, label)
hwmon_temp() {
  local want_name="$1" want_lbl="$2"
  for d in /sys/class/hwmon/hwmon*; do
    [ -d "$d" ] || continue
    [ "$(cat "$d/name" 2>/dev/null)" = "$want_name" ] || continue
    for t in "$d"/temp*_input; do
      [ -r "$t" ] || continue
      local lf="${t%_input}_label"
      local got=""
      [ -r "$lf" ] && got=$(cat "$lf" 2>/dev/null)
      if [ "$want_lbl" = "*" ] || [ "$got" = "$want_lbl" ]; then
        awk '{printf "%.1f", $1/1000}' "$t"
        return
      fi
    done
  done
  echo "?"
}

# read first hwmon fan we can find by name
hwmon_fan() {
  local want_name="$1" idx="${2:-1}"
  for d in /sys/class/hwmon/hwmon*; do
    [ -d "$d" ] || continue
    [ "$(cat "$d/name" 2>/dev/null)" = "$want_name" ] || continue
    local f="$d/fan${idx}_input"
    [ -r "$f" ] && { cat "$f"; return; }
  done
  echo "?"
}

# --- panel renderers --------------------------------------------------------

panel_thermal() {
  # Pull the four interesting probes; °C scale, color-graded.
  local cpu gpu ram amb nvme fan
  cpu=$(hwmon_temp cros_ec "cpu@4c")
  [ "$cpu" = "?" ] && cpu=$(hwmon_temp acpitz "*")   # fallback any acpi temp
  gpu=$(hwmon_temp amdgpu edge)
  ram=$(hwmon_temp cros_ec "mainboard_memory@4d")
  amb=$(hwmon_temp cros_ec "mainboard_ambient@4d")
  nvme=$(hwmon_temp nvme "Composite")
  fan=$(hwmon_fan cros_ec 1)

  local lines=""
  emit_t() {
    local lbl="$1" v="$2" max="$3" warn="$4" hot="$5" w="$6"
    local col
    col=$(pick_color "$v" "$warn" "$hot")
    lines+="$(printf '%-6s %s%5s°C%s  %s' \
              "$lbl" "$col" "$v" "$C_RESET" "$(bar "$v" "$max" "$w")")"$'\n'
  }
  emit_t "CPU"   "$cpu"  100 65 85 16
  emit_t "GPU"   "$gpu"  100 65 85 16
  emit_t "RAM"   "$ram"   80 55 70 16
  emit_t "amb"   "$amb"   60 45 55 16
  emit_t "NVMe"  "$nvme"  80 55 70 16
  if [ "$fan" != "?" ]; then
    local fc; fc=$(pick_color "$fan" 1500 3000)
    lines+="$(printf '%-6s %s%5s rpm%s %s' \
              "fan1" "$fc" "$fan" "$C_RESET" "$(bar "$fan" 4000 15)")"
  fi
  printf '%s' "${lines%$'\n'}" | gum style \
    --border rounded --border-foreground 244 \
    --padding "0 1" --margin "0" \
    --foreground 252 \
    --width 50
}

panel_gpu() {
  # iGPU (amdgpu) — busy %, package power (PPT on APUs), VRAM, edge temp.
  local lines=""
  if [ -z "$GPU_DEV" ]; then
    lines="  ${c_dim}(no amdgpu sysfs surface found)${C_RESET}"
  else
    local busy pwr vu vt edge vram_pct
    busy=$(cat "$GPU_DEV/gpu_busy_percent" 2>/dev/null || echo '?')
    pwr='?'; [ -n "$GPU_PWR" ] && pwr="$(microw_to_w < "$GPU_PWR")"
    vu='?'; vt='?'
    if [ -r "$GPU_DEV/mem_info_vram_used" ]; then
      vu="$(bytes_to_gb < "$GPU_DEV/mem_info_vram_used")"
      vt="$(bytes_to_gb < "$GPU_DEV/mem_info_vram_total")"
    fi
    edge=$(hwmon_temp amdgpu edge)
    local busy_col pwr_col edge_col
    busy_col=$(pick_color "$busy" 30 70)
    pwr_col=$(pick_color "$pwr" 45 90)
    edge_col=$(pick_color "$edge" 65 85)
    if [ "$vu" != "?" ] && [ "$vt" != "?" ]; then
      vram_pct=$(awk -v u="$vu" -v t="$vt" 'BEGIN{if(t+0==0)print 0;else printf "%.0f", u/t*100}')
    else
      vram_pct=0
    fi
    lines+="$(printf 'busy    %s%5s%%%s  %s' \
              "$busy_col" "$busy" "$C_RESET" "$(bar "$busy" 100 18)")"$'\n'
    lines+="$(printf '%-7s %s%5s W%s  %s' \
              "$GPU_PWR_LABEL" "$pwr_col" "$pwr" "$C_RESET" "$(bar "$pwr" 120 18)")"$'\n'
    lines+="$(printf 'vram   %s%4s/%s GB%s %s' \
              "$c_cyan" "$vu" "$vt" "$C_RESET" "$(bar "$vram_pct" 100 14)")"$'\n'
    lines+="$(printf 'edge   %s%5s°C%s   %s' \
              "$edge_col" "$edge" "$C_RESET" "$(bar "$edge" 100 18)")"
  fi
  printf '%s' "$lines" | gum style \
    --border rounded --border-foreground 244 \
    --padding "0 1" --margin "0" \
    --foreground 252 \
    --width 50
}

panel_cpu() {
  # load (1/5/15) + user/sys/idle split + top 3 consumers.
  local load1 load5 load15 cores
  read -r load1 load5 load15 _ <<<"$(awk '{print $1, $2, $3}' /proc/loadavg) _"
  cores=$(nproc)

  local user sys idle iowait
  read -r user sys idle iowait <<<"$(top -bn1 2>/dev/null | awk -F'[, ]+' '/Cpu\(s\)/ {
    for (i=1;i<=NF;i++) {
      if ($i=="us") us=$(i-1)
      if ($i=="sy") sy=$(i-1)
      if ($i=="id") id=$(i-1)
      if ($i=="wa") wa=$(i-1)
    }
    printf "%s %s %s %s", us, sy, id, wa
  }')"

  local load_pct
  load_pct=$(awk -v l="$load1" -v c="$cores" 'BEGIN{printf "%.0f", l/c*100}')
  local load_col idle_col user_col
  load_col=$(pick_color "$load_pct" 50 80)
  idle_col=$(pick_color "$((100 - ${idle%.*}))" 50 80)
  user_col=$(pick_color "${user%.*}" 40 70)

  local lines=""
  lines+="$(printf 'load   %s%5s%s  %s' \
            "$load_col" "$load1" "$C_RESET" "$(bar "$load_pct" 100 18)")"$'\n'
  lines+="$(printf '%s%s · %s · %s (1·5·15m avg, %s cores)%s' \
            "$c_dim" "$load1" "$load5" "$load15" "$cores" "$C_RESET")"$'\n'
  lines+="$(printf 'user   %s%4s%%%s  sys %s%s%%%s  idle %s%s%%%s' \
            "$user_col" "$user" "$C_RESET" \
            "$c_dim" "$sys"  "$C_RESET" \
            "$idle_col" "$idle" "$C_RESET")"$'\n'

  # top 3 by CPU
  lines+="${c_dim}top by cpu%${C_RESET}"$'\n'
  local count=0
  while read -r pcpu comm; do
    count=$((count+1))
    [ $count -gt 3 ] && break
    local col
    col=$(pick_color "${pcpu%.*}" 30 80)
    lines+="$(printf '    %s%5s%%%s  %s' "$col" "$pcpu" "$C_RESET" "$comm")"$'\n'
  done < <(ps -eo pcpu,comm --sort=-pcpu --no-headers 2>/dev/null | head -3)

  printf '%s' "${lines%$'\n'}" | gum style \
    --border rounded --border-foreground 244 \
    --padding "0 1" --margin "0" \
    --foreground 252 \
    --width 50
}

panel_memory() {
  # RAM (used / total / cache / swap) read from /proc/meminfo.
  local total free avail buffers cached swap_total swap_free
  total=$(awk '/^MemTotal:/{print $2}' /proc/meminfo)
  free=$(awk '/^MemFree:/{print $2}' /proc/meminfo)
  avail=$(awk '/^MemAvailable:/{print $2}' /proc/meminfo)
  buffers=$(awk '/^Buffers:/{print $2}' /proc/meminfo)
  cached=$(awk '/^Cached:/{print $2}' /proc/meminfo)
  swap_total=$(awk '/^SwapTotal:/{print $2}' /proc/meminfo)
  swap_free=$(awk '/^SwapFree:/{print $2}' /proc/meminfo)
  local used pct cache_gb swap_used swap_pct
  used=$((total - avail))                          # "really used" = total - available
  pct=$(awk -v u="$used" -v t="$total" 'BEGIN{printf "%.0f", u/t*100}')
  cache_gb=$(echo "$cached" | kb_to_gb)
  swap_used=$((swap_total - swap_free))
  swap_pct=$(awk -v u="$swap_used" -v t="$swap_total" 'BEGIN{if(t+0==0)print 0;else printf "%.0f", u/t*100}')

  local used_gb total_gb free_gb swap_used_gb swap_total_gb
  used_gb=$(echo "$used"       | kb_to_gb)
  total_gb=$(echo "$total"     | kb_to_gb)
  free_gb=$(echo "$free"       | kb_to_gb)
  swap_used_gb=$(echo "$swap_used"   | kb_to_gb)
  swap_total_gb=$(echo "$swap_total" | kb_to_gb)

  local ram_col swap_col
  ram_col=$(pick_color "$pct" 60 85)
  swap_col=$(pick_color "$swap_pct" 10 50)

  local lines=""
  lines+="$(printf 'used   %s%5s%%%s  %s' \
            "$ram_col" "$pct" "$C_RESET" "$(bar "$pct" 100 18)")"$'\n'
  lines+="$(printf '%s%s / %s GB used%s' \
            "$c_dim" "$used_gb" "$total_gb" "$C_RESET")"$'\n'
  lines+="$(printf 'cache  %s%5s GB%s  %sfree %s GB%s' \
            "$c_cyan" "$cache_gb" "$C_RESET" "$c_dim" "$free_gb" "$C_RESET")"$'\n'
  if [ "${swap_total:-0}" -gt 0 ]; then
    lines+="$(printf 'swap   %s%5s%%%s  %s%s/%s GB%s' \
              "$swap_col" "$swap_pct" "$C_RESET" \
              "$c_dim" "$swap_used_gb" "$swap_total_gb" "$C_RESET")"
  else
    lines+="${c_dim}swap   (none)${C_RESET}"
  fi
  printf '%s' "$lines" | gum style \
    --border rounded --border-foreground 244 \
    --padding "0 1" --margin "0" \
    --foreground 252 \
    --width 50
}

panel_ollama() {
  local lines="" data
  data=$(curl -sS --max-time 2 http://localhost:11434/api/ps 2>/dev/null | python3 -c '
import json, sys
try: d=json.load(sys.stdin)
except Exception: sys.exit(0)
for m in d.get("models", []):
  name=m.get("name","?")
  mb=m.get("size_vram",0)//1024//1024
  exp=(m.get("expires_at","") or "")[:19].replace("T"," ")
  print(f"{name}\t{mb}\t{exp}")
' 2>/dev/null)
  if [ -z "$data" ]; then
    lines="  ${c_dim}(ollama unreachable on :11434 — no models loaded)${C_RESET}"
  else
    local count=0
    while IFS=$'\t' read -r name mb exp; do
      [ -z "$name" ] && continue
      count=$((count+1))
      local col
      col=$(pick_color "$mb" 3000 6000)
      lines+="$(printf '%-32s %s%6s MB%s VRAM   %sexp %s%s' \
                "$name" "$col" "$mb" "$C_RESET" "$c_dim" "$exp" "$C_RESET")"$'\n'
    done <<< "$data"
    [ "$count" = 0 ] && lines="  ${c_dim}(no models loaded — ollama responded with empty list)${C_RESET}"
  fi
  printf '%s' "${lines%$'\n'}" | gum style \
    --border rounded --border-foreground 244 \
    --padding "0 1" --margin "0" \
    --foreground 252 \
    --width 102
}

header() {
  local host now
  host=$(hostname -s 2>/dev/null || uname -n)
  now=$(date '+%Y-%m-%d %H:%M:%S')
  gum style \
    --border double --border-foreground 87 \
    --padding "0 2" --margin "0" --align center \
    --foreground 87 --bold \
    --width 102 \
    "hex HUD   ·   ${host}   ·   ${now}   ·   refresh ${INTERVAL}s"
}

legend() {
  # Brief inline glossary — explains the non-obvious labels.
  printf '%s' "  \
${c_bold}legend${C_RESET} ${c_dim}·${C_RESET} \
${c_cyan}PPT${C_RESET}=APU package power (CPU+iGPU+uncore) ${c_dim}·${C_RESET} \
${c_cyan}busy${C_RESET}=GPU compute % ${c_dim}·${C_RESET} \
${c_cyan}edge${C_RESET}=iGPU die temp ${c_dim}·${C_RESET} \
${c_cyan}load${C_RESET}=Linux runqueue (1·5·15m) ${c_dim}·${C_RESET} \
${c_cyan}used${C_RESET}=RAM minus reclaimable cache"
  printf '\n'
  printf '%sCtrl-C quit · NO_COLOR=1 strip color · INTERVAL=N refresh rate%s\n' \
    "$c_dim" "$C_RESET"
}

render_once() {
  local frame
  frame="$(header)"$'\n'
  frame+="$(gum join --horizontal "$(panel_thermal)" "$(panel_gpu)")"$'\n'
  frame+="$(gum join --horizontal "$(panel_cpu)" "$(panel_memory)")"$'\n'
  frame+="$(panel_ollama)"$'\n'
  frame+="$(legend)"
  printf '\033[H%s\033[J' "$frame"
}

if [ -t 1 ]; then
  printf '\033[?1049h\033[?25l'
  trap 'printf "\033[?25h\033[?1049l"; exit 0' INT TERM EXIT
  while true; do
    render_once
    sleep "$INTERVAL"
  done
else
  render_once
fi
