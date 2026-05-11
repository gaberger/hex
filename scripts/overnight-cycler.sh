#!/bin/bash
# Overnight autonomy cycler. Runs until 06:00 EDT 2026-05-11.
# Every 75 min: commit clean landed work, fire next batch of asks.
# Final wake at 06:00 EDT compiles report inputs.

set +e
TARGET_EPOCH=$(date -d "2026-05-11 06:00:00 EDT" +%s)
LOG=/tmp/overnight_cycler.log
PROMPTS=/tmp/overnight_prompts.d
mkdir -p $PROMPTS

export PATH=$HOME/.cargo/bin:$PATH
cd /var/home/gary/hex-intf

echo "=== overnight cycler started $(date) target=$(date -d @$TARGET_EPOCH) ===" > $LOG

# Generic recurring asks (rotate through these)
generate_prompts() {
  local cycle=$1
  cat > $PROMPTS/ca-$cycle.json <<JSONEOF
{"from": "ceo", "content": "@chief-architect intent=code_patch — Overnight cycle $cycle. Find another structural improvement — a missing port, a dead-code cluster, a boundary violation, a tool that should be split into two, or a tool that should be merged. Run workspace_boundary_check and act on output. Ship a code_patch. If nothing actionable, draft an ADR for a future structural concern. Don't ask permission."}
JSONEOF
  cat > $PROMPTS/cto-$cycle.json <<JSONEOF
{"from": "ceo", "content": "@cto intent=code_patch — Overnight cycle $cycle. Scan recent commits for opportunities: TODO comments to resolve, unused warnings to address, error-message clarity improvements, retry logic for fire-and-forget paths. Pick one, ship code_patch. Don't ask permission."}
JSONEOF
  cat > $PROMPTS/cv-$cycle.json <<JSONEOF
{"from": "ceo", "content": "@chief-visionary intent=spec_draft — Overnight cycle $cycle. Draft a small forward-looking spec (50-200 lines) on one of: (a) inter-substrate composability (how hex talks to OTHER hex instances), (b) the operator-AI feedback loop (how the operator's memory should update from observed persona behavior), (c) a paradigm shift the team hasn't explored. Don't ask permission — pick and write."}
JSONEOF
  cat > $PROMPTS/ciso-$cycle.json <<JSONEOF
{"from": "ceo", "content": "@ciso intent=code_question — Overnight cycle $cycle. Run secret_scan + dep_audit again and diff against last run's findings. Report any new entries, regressions, or environmental drift. KEEP UNDER 1500 CHARS."}
JSONEOF
  cat > $PROMPTS/cpo-$cycle.json <<JSONEOF
{"from": "ceo", "content": "@cpo intent=spec_draft — Overnight cycle $cycle. Write a 100-200 line spec for one observable improvement to the operator surface — a new Mission Control panel, a CLI affordance, a status badge, a notification format. Don't ask permission, draft + ship."}
JSONEOF
  cat > $PROMPTS/coo-$cycle.json <<JSONEOF
{"from": "ceo", "content": "@coo intent=spec_draft — Overnight cycle $cycle. Document one operational ritual or audit cadence as a spec (50-200 lines): morning health check, weekly cost review, monthly ADR-status sweep, quarterly persona-performance review. Reference cost-ops-runbook.md + coo-observability-baseline.md. Don't ask permission."}
JSONEOF
}

CYCLE=0
while [ $(date +%s) -lt $TARGET_EPOCH ]; do
  CYCLE=$((CYCLE + 1))
  echo "[$(date)] cycle $CYCLE begin" >> $LOG

  # Sleep 75 min between batches (4500s) - lets prior batch drain
  # Skip sleep on first iteration to allow seed asks to drain
  if [ $CYCLE -gt 1 ]; then
    sleep 4500
  else
    # First batch sleep: 60 min to let seed (12 asks) drain
    sleep 3600
  fi

  # Commit anything new that landed
  cd /var/home/gary/hex-intf
  if [ -n "$(git status --porcelain | grep -v '^??' | head -1)" ] || git ls-files --others --exclude-standard | grep -q "docs/adrs/\|docs/specs/\|hex-.*/src/tools/" ; then
    # Auto-stage docs/adrs, docs/specs, new tools/adapters, and modified Rust source
    git add docs/adrs/*.md docs/specs/*.md 2>/dev/null
    git add hex-nexus/src/tools/*.rs hex-cli/src/commands/*.rs hex-nexus/src/adapters/*.rs hex-nexus/src/orchestration/*.rs 2>/dev/null
    git add hex-nexus/src/tools/mod.rs hex-nexus/src/adapters/mod.rs 2>/dev/null
    # Verify build before committing
    if cargo check -p hex-nexus -p hex-cli >>$LOG 2>&1; then
      git commit -m "overnight cycle $CYCLE autonomous shipments (operator asleep)" >>$LOG 2>&1
      # Rebuild + restart nexus to pick up changes
      cargo build --release -p hex-nexus >>$LOG 2>&1
      if [ -f target/x86_64-unknown-linux-gnu/release/hex-nexus ]; then
        hex nexus stop >>$LOG 2>&1
        sleep 2
        cp target/x86_64-unknown-linux-gnu/release/hex-nexus ~/.local/bin/hex-nexus
        export HEX_DISABLE_WORKPLAN_AUTO_EMITTER=1
        export HEX_SOP_PERSONAS="cto,cpo,coo,ciso,chief-visionary,chief-architect"
        export HEX_COST_WATCHDOG_INTERVAL_SECS=60
        export HEX_SOP_MAX_TOKENS=8192
        hex nexus start >>$LOG 2>&1
        until curl -fsS http://127.0.0.1:5555/api/health >/dev/null 2>&1; do sleep 1; done
      fi
    else
      echo "[$(date)] cycle $CYCLE BUILD FAILED, skipping commit" >> $LOG
      git stash >> $LOG 2>&1
    fi
  fi

  # Stop firing new asks 75 min before target so the queue can drain
  if [ $(($(date +%s) + 4500)) -lt $TARGET_EPOCH ]; then
    generate_prompts $CYCLE
    for f in $PROMPTS/*-$CYCLE.json; do
      curl -sS -X POST http://127.0.0.1:5555/api/org/send-message \
        -H "Content-Type: application/json" \
        --data-binary "@$f" >>$LOG 2>&1
      echo " (fired $(basename $f))" >> $LOG
    done
  fi

  echo "[$(date)] cycle $CYCLE end. SOP runs total: $(grep -c 'SOP run complete' /home/gary/.hex/nexus.log)" >> $LOG
done

echo "=== overnight cycler done $(date) — woke up for report ===" >> $LOG
