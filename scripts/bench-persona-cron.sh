#!/usr/bin/env bash
# Persona benchmark cron entry.
#
# Suggested schedule:
#   0 3 * * *    daily local sweep                  (~5min, free)
#   0 6 * * 1    weekly OpenRouter sweep on Monday  (~3min, ~$0.05-$2)
#
# Install with:
#   crontab -l > /tmp/c && \
#   echo "0 3 * * * /var/home/gary/hex-intf/scripts/bench-persona-cron.sh daily >> /var/home/gary/.hex/bench-cron.log 2>&1" >> /tmp/c && \
#   echo "0 6 * * 1 /var/home/gary/hex-intf/scripts/bench-persona-cron.sh weekly >> /var/home/gary/.hex/bench-cron.log 2>&1" >> /tmp/c && \
#   crontab /tmp/c && rm /tmp/c
#
# Or use systemd-timer if you prefer. Either is fine — hex's own sched
# daemon doesn't yet have a verb for this (could add one later).

set -euo pipefail

mode="${1:-daily}"
repo="/var/home/gary/hex-intf"
out_dir="$repo/docs/analysis/bench"
mkdir -p "$out_dir"
stamp="$(date +%Y-%m-%d)"

cd "$repo"

case "$mode" in
  daily)
    out="$out_dir/persona-ollama-$stamp.json"
    echo "[$(date -Iseconds)] daily local Ollama sweep → $out"
    python3 scripts/bench-persona-prompts.py --json > "$out"
    ;;
  weekly)
    out="$out_dir/persona-openrouter-$stamp.json"
    echo "[$(date -Iseconds)] weekly OpenRouter sweep → $out"
    # Cover the major families: claude / gpt / gemini / deepseek + a free-tier scan
    python3 scripts/bench-persona-prompts.py --provider openrouter \
        --model \
          anthropic/claude-sonnet-4 \
          anthropic/claude-3.5-haiku \
          openai/gpt-4o-mini \
          openai/gpt-4o \
          google/gemini-2.5-flash \
          google/gemini-2.0-flash-001 \
          deepseek/deepseek-chat \
          mistralai/mistral-large \
        --json > "$out"
    ;;
  prompt-regression)
    # Run after any change to persona_prompt / conversational_prompt to
    # verify the current default model still passes. Exits non-zero if
    # the average score drops below 0.85.
    out="$out_dir/persona-regression-$(date +%s).json"
    echo "[$(date -Iseconds)] prompt-regression check → $out"
    python3 scripts/bench-persona-prompts.py --json > "$out"
    avg="$(python3 -c "
import json, sys
d = json.load(open('$out'))
totals = {}
for m, runs in d['results'].items():
    s = [v.get('score', 0) for v in runs.values()]
    totals[m] = sum(s)/len(s) if s else 0
top = max(totals.items(), key=lambda x: x[1])
print(f'{top[0]} {top[1]:.3f}')
")"
    score="${avg##* }"
    model="${avg% *}"
    echo "winning model: $model  score: $score"
    if python3 -c "import sys; sys.exit(0 if float('$score') >= 0.85 else 1)"; then
        echo "✓ regression check pass"
        exit 0
    else
        echo "✗ regression check fail — best model below 0.85 threshold"
        exit 1
    fi
    ;;
  *)
    echo "usage: $0 {daily|weekly|prompt-regression}"
    exit 2
    ;;
esac

# Retention: keep last 90 days of bench archives, prune older.
find "$out_dir" -name 'persona-*.json' -mtime +90 -delete

# Surface a tiny summary line for the cron log
python3 -c "
import json, sys
try:
    d = json.load(open('$out'))
    results = d.get('results', {})
    avgs = {m: sum(v.get('score',0) for v in r.values())/max(1,len(r))
            for m, r in results.items()}
    top = sorted(avgs.items(), key=lambda x: -x[1])[:3]
    print(f'$mode top 3:')
    for m, s in top:
        print(f'  {s:.2f}  {m}')
except Exception as e:
    print(f'summary failed: {e}')
"
