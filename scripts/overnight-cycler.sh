#!/bin/bash
# Overnight autonomy cycler — ADR-2605110700 R2/R3/R4 hardened.
#
# R1 (hard-gate cargo_check) lives in hex-nexus/src/orchestration/action_executor.rs.
# R2: per-file commit, not per-cycle atomic — docs ship even if Rust breaks.
# R3: don't stash on failure — leave broken state; R1 prevents worse.
# R4: wall-clock watchdog — checks date >= target at top of loop, bounded sleeps.

set +e
TARGET_DT="${1:-$(date -d 'tomorrow 06:00' '+%Y-%m-%d %H:%M:%S')}"
TARGET_EPOCH=$(date -d "$TARGET_DT" +%s)
LOG=/tmp/overnight_cycler.log
PROMPTS=/tmp/overnight_prompts.d
mkdir -p $PROMPTS

export PATH=$HOME/.cargo/bin:/usr/bin:/bin:$PATH
cd "${HEX_REPO:-/home/gary/hex-intf}"

echo "=== overnight cycler started $(date) target=$(date -d @$TARGET_EPOCH) ===" > $LOG
echo "PID $$ — R2/R3/R4 hardened" >> $LOG

generate_prompts() {
  local cycle=$1
  cat > $PROMPTS/ca-$cycle.json <<JSONEOF
{"from": "ceo", "content": "@chief-architect intent=code_patch — Overnight cycle $cycle. R1 hard-gate is live so broken patches roll back automatically. Find structural improvement, ship via code_patch end-to-end. If patch rolls back, try different approach — mark_failed evidence trail tells you what broke. Don't ask permission."}
JSONEOF
  cat > $PROMPTS/cto-$cycle.json <<JSONEOF
{"from": "ceo", "content": "@cto intent=code_patch — Overnight cycle $cycle. R1 hard-gate live. Improvement candidates: TODO comments, dead-code warnings, retry logic for fire-and-forget paths, error-message clarity. Pick one, ship code_patch. Don't ask permission."}
JSONEOF
  cat > $PROMPTS/cv-$cycle.json <<JSONEOF
{"from": "ceo", "content": "@chief-visionary intent=spec_draft — Overnight cycle $cycle. Draft small forward-looking spec (50-200 lines) on inter-substrate composability, operator-AI feedback loops, or an unexplored paradigm. Don't ask permission."}
JSONEOF
  cat > $PROMPTS/ciso-$cycle.json <<JSONEOF
{"from": "ceo", "content": "@ciso intent=code_question — Overnight cycle $cycle. Run secret_scan + dep_audit and diff against last run. Report new entries, regressions, env drift. KEEP UNDER 1500 CHARS."}
JSONEOF
  cat > $PROMPTS/cpo-$cycle.json <<JSONEOF
{"from": "ceo", "content": "@cpo intent=spec_draft — Overnight cycle $cycle. Spec one observable operator-surface improvement (100-200 lines): Mission Control panel, CLI affordance, status badge, notification format. Don't ask permission."}
JSONEOF
  cat > $PROMPTS/coo-$cycle.json <<JSONEOF
{"from": "ceo", "content": "@coo intent=spec_draft — Overnight cycle $cycle. Document one operational ritual or audit cadence (50-200 lines). Reference cost-ops-runbook.md + coo-observability-baseline.md + your prior standup specs. Don't ask permission."}
JSONEOF
}

CYCLE=0
while true; do
  NOW=$(date +%s)
  if [ $NOW -ge $TARGET_EPOCH ]; then
    echo "[$(date)] target reached — exiting" >> $LOG
    break
  fi

  CYCLE=$((CYCLE + 1))
  echo "[$(date)] cycle $CYCLE begin" >> $LOG

  # R4: bounded sleep — never sleep past target
  if [ $CYCLE -gt 1 ]; then
    REMAINING=$((TARGET_EPOCH - NOW))
    SLEEP_FOR=$((REMAINING < 4500 ? REMAINING : 4500))
  else
    REMAINING=$((TARGET_EPOCH - NOW))
    SLEEP_FOR=$((REMAINING < 3600 ? REMAINING : 3600))
  fi
  if [ $SLEEP_FOR -gt 0 ]; then
    echo "[$(date)] sleeping ${SLEEP_FOR}s" >> $LOG
    sleep $SLEEP_FOR
  fi

  cd "${HEX_REPO:-/home/gary/hex-intf}"

  # R2: per-file commit. Docs always safe to commit.
  DOCS_CHANGED=0
  for path in $(git status --porcelain | grep -E "^(\?\?|.M|MM) docs/" | awk '{print $2}'); do
    git add "$path" >>$LOG 2>&1
    DOCS_CHANGED=1
  done
  if [ $DOCS_CHANGED -eq 1 ]; then
    git commit -m "overnight cycle $CYCLE docs (autonomous, build-safe)" >>$LOG 2>&1
    echo "[$(date)] cycle $CYCLE docs committed" >> $LOG
  fi

  # Rust per-crate. R3: no stash; R1 protects against accumulation.
  for crate in hex-nexus hex-cli hex-core hex-agent hex-analyzer hex-parser; do
    CHANGED_FILES=$(git status --porcelain | grep -E "^(\?\?|.M|MM) ${crate}/" | awk '{print $2}')
    if [ -z "$CHANGED_FILES" ]; then continue; fi
    if cargo check -p $crate >>$LOG 2>&1; then
      for f in $CHANGED_FILES; do git add "$f" >>$LOG 2>&1; done
      git commit -m "overnight cycle $CYCLE $crate (autonomous, cargo_check passed)" >>$LOG 2>&1
      echo "[$(date)] cycle $CYCLE $crate committed" >> $LOG
    else
      echo "[$(date)] cycle $CYCLE $crate BUILD FAILED — leaving state (R1 protects)" >> $LOG
    fi
  done

  # Rebuild + restart nexus if hex-nexus has recent commits
  if git log --since="2 minutes ago" --oneline 2>/dev/null | grep -qE "hex-nexus|R1|R2|R3|R4|fix\(executor"; then
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
  fi

  # R4: stop firing in last 75 min before target
  NOW=$(date +%s)
  if [ $((NOW + 4500)) -lt $TARGET_EPOCH ]; then
    generate_prompts $CYCLE
    for f in $PROMPTS/*-$CYCLE.json; do
      curl -sS -X POST http://127.0.0.1:5555/api/org/send-message \
        -H "Content-Type: application/json" \
        --data-binary "@$f" >>$LOG 2>&1
      echo " (fired $(basename $f))" >> $LOG
    done
  else
    echo "[$(date)] cycle $CYCLE — within drain-window, no new asks" >> $LOG
  fi

  echo "[$(date)] cycle $CYCLE end. SOP runs total: $(grep -c 'SOP run complete' /home/gary/.hex/nexus.log 2>/dev/null)" >> $LOG
done

echo "=== overnight cycler done $(date) — exited cleanly at target ===" >> $LOG
