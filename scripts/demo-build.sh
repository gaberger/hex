#!/usr/bin/env bash
# demo-build.sh — Full hex dev pipeline demo: prompt → code → compile → run
#
# Runs `hex dev start <prompt> --auto`, then verifies the generated application
# actually compiles (and optionally passes its own tests).
#
# Usage:
#   ./scripts/demo-build.sh                                 # default: Go hello world
#   ./scripts/demo-build.sh "build a URL shortener in Go"  # custom prompt
#   PROVIDER=anthropic ./scripts/demo-build.sh             # different provider
#
# Requirements:
#   - hex CLI in PATH  (cargo build -p hex-cli --release)
#   - hex-nexus running or auto-started
#   - Inference provider: OPENROUTER_API_KEY (or ANTHROPIC_API_KEY with PROVIDER=anthropic)

set -euo pipefail

# ── Config ────────────────────────────────────────────────────────────────────

PROMPT="${1:-build a hello world CLI in Go that prints Hello, hex!}"
PROVIDER="${PROVIDER:-openrouter}"
TIMEOUT_SECS="${TIMEOUT_SECS:-300}"
OUTPUT_BASE="$(pwd)/examples"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

PASS=0
FAIL=0
STARTED_NEXUS=false
NEXUS_PID=""

cleanup() {
    if [ "$STARTED_NEXUS" = true ] && [ -n "$NEXUS_PID" ]; then
        kill "$NEXUS_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

pass()  { echo -e "  ${GREEN}✓${NC} $1"; PASS=$((PASS+1)); }
fail()  { echo -e "  ${RED}✗${NC} $1"; FAIL=$((FAIL+1)); }
info()  { echo -e "  ${CYAN}→${NC} $1"; }
warn()  { echo -e "  ${YELLOW}!${NC} $1"; }
fatal() { echo -e "\n  ${RED}${BOLD}FATAL:${NC} $1\n"; exit 1; }

echo -e "${CYAN}${BOLD}⬡ hex — full build demo${NC}"
echo -e "${CYAN}──────────────────────────────────────────────────${NC}"
echo -e "  Prompt:   ${BOLD}${PROMPT}${NC}"
echo -e "  Provider: ${PROVIDER}"
echo -e "  Timeout:  ${TIMEOUT_SECS}s"
echo -e "${CYAN}──────────────────────────────────────────────────${NC}"
echo ""

# ── Phase 1: Prerequisites ────────────────────────────────────────────────────

echo -e "${CYAN}── Phase 1: Prerequisites ──${NC}"

if ! command -v hex &>/dev/null; then
    fatal "hex binary not found\n  Build it: cargo build -p hex-cli --release\n  Install:  cp target/release/hex /usr/local/bin/hex"
fi
pass "hex binary ($(hex --version 2>/dev/null | head -1 || echo 'unknown version'))"

# Start nexus if not already running
if ! curl -sf http://127.0.0.1:5555/api/health >/dev/null 2>&1; then
    info "hex-nexus not running — starting..."
    hex nexus start &
    NEXUS_PID=$!
    STARTED_NEXUS=true
    for i in $(seq 1 10); do
        sleep 1
        if curl -sf http://127.0.0.1:5555/api/health >/dev/null 2>&1; then
            break
        fi
        if [ "$i" -eq 10 ]; then
            fatal "hex-nexus failed to start after 10s"
        fi
    done
    pass "hex-nexus started (PID $NEXUS_PID)"
else
    pass "hex-nexus running"
fi

echo ""

# ── Phase 2: Run hex dev pipeline ────────────────────────────────────────────

echo -e "${CYAN}── Phase 2: hex dev pipeline ──${NC}"
info "Running: hex dev start \"${PROMPT}\" --auto --provider ${PROVIDER}"
echo ""

START_TS=$(date +%s)
if ! timeout "$TIMEOUT_SECS" hex dev start "$PROMPT" --auto --provider "$PROVIDER"; then
    echo ""
    fatal "hex dev pipeline exited non-zero or timed out after ${TIMEOUT_SECS}s"
fi
END_TS=$(date +%s)
ELAPSED=$((END_TS - START_TS))

echo ""
pass "Pipeline completed in ${ELAPSED}s"

# ── Phase 3: Locate output directory ─────────────────────────────────────────

echo ""
echo -e "${CYAN}── Phase 3: Output verification ──${NC}"

# Derive slug the same way hex does (lowercase, spaces→hyphens, collapse runs)
SLUG=$(echo "$PROMPT" | tr '[:upper:]' '[:lower:]' \
    | sed 's/[^a-z0-9]/-/g' \
    | sed 's/--*/-/g' \
    | sed 's/^-//;s/-$//')
OUTPUT_DIR="${OUTPUT_BASE}/${SLUG}"

if [ ! -d "$OUTPUT_DIR" ]; then
    # Fallback: most recently modified directory under examples/
    LATEST=$(ls -td "${OUTPUT_BASE}"/*/  2>/dev/null | head -1 || echo "")
    if [ -n "$LATEST" ]; then
        OUTPUT_DIR="${LATEST%/}"
        warn "Expected '${SLUG}' not found — using most recent: $(basename "$OUTPUT_DIR")"
    else
        fatal "No output directory found under ${OUTPUT_BASE}"
    fi
fi
pass "Output: $(basename "$OUTPUT_DIR")"

# Expected scaffolding files
[ -f "${OUTPUT_DIR}/README.md" ] && pass "README.md" || warn "README.md missing"
[ -f "${OUTPUT_DIR}/start.sh" ]  && pass "start.sh"  || warn "start.sh missing"

# ── Phase 4: Compile check (language-aware) ───────────────────────────────────

echo ""
echo -e "${CYAN}── Phase 4: Compile check ──${NC}"

if [ -f "${OUTPUT_DIR}/go.mod" ]; then
    info "Go project detected"
    if (cd "$OUTPUT_DIR" && go build ./... 2>&1 | tail -5); then
        pass "go build ./..."
    else
        fail "go build ./..."
    fi
    # Run tests if any exist
    if find "$OUTPUT_DIR" -name "*_test.go" | grep -q .; then
        if (cd "$OUTPUT_DIR" && go test ./... 2>&1 | tail -5); then
            pass "go test ./..."
        else
            fail "go test ./..."
        fi
    fi

elif [ -f "${OUTPUT_DIR}/Cargo.toml" ]; then
    info "Rust project detected"
    if (cd "$OUTPUT_DIR" && cargo check 2>&1 | tail -5); then
        pass "cargo check"
    else
        fail "cargo check"
    fi
    if (cd "$OUTPUT_DIR" && cargo test 2>&1 | tail -5); then
        pass "cargo test"
    else
        fail "cargo test"
    fi

elif [ -f "${OUTPUT_DIR}/package.json" ]; then
    info "Node.js project detected"
    if (cd "$OUTPUT_DIR" && npm install --silent 2>&1 | tail -3); then
        pass "npm install"
    else
        fail "npm install"
    fi
    if (cd "$OUTPUT_DIR" && npm run build 2>&1 | tail -5); then
        pass "npm run build"
    else
        fail "npm run build"
    fi

else
    warn "Unknown language — skipping compile check"
fi

# ── Summary ───────────────────────────────────────────────────────────────────

echo ""
echo -e "${CYAN}══════════════════════════════════════════════════${NC}"
TOTAL=$((PASS + FAIL))

if [ "$FAIL" -eq 0 ]; then
    echo -e "  ${GREEN}${BOLD}DEMO PASSED${NC}  ($PASS/$TOTAL checks)"
    echo -e "  Built: ${BOLD}$(basename "$OUTPUT_DIR")${NC} in ${ELAPSED}s"
    echo -e "${CYAN}══════════════════════════════════════════════════${NC}"
    echo ""
    echo -e "  Try it:  ${BOLD}cd ${OUTPUT_DIR} && bash start.sh${NC}"
    echo ""
    exit 0
else
    echo -e "  ${RED}${BOLD}DEMO FAILED${NC}  ($PASS passed, $FAIL failed of $TOTAL)"
    echo -e "${CYAN}══════════════════════════════════════════════════${NC}"
    exit 1
fi
