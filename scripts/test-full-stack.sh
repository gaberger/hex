#!/usr/bin/env bash
# test-full-stack.sh — Automated end-to-end test for the hex system
#
# Tests: build → unit tests → start services → health checks → agent spawn → cleanup
#
# Usage:
#   ./scripts/test-full-stack.sh           # Full stack test
#   ./scripts/test-full-stack.sh --quick   # Unit tests only (no services)

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
NC='\033[0m'

PASS=0
FAIL=0
SKIP=0
NEXUS_PID=""
CHAT_PID=""
NEXUS_PORT=15555  # Use non-default port to avoid conflicting with running instances
CHAT_PORT=15556

cleanup() {
    echo -e "\n${CYAN}── Cleanup ──${NC}"
    [ -n "$NEXUS_PID" ] && kill "$NEXUS_PID" 2>/dev/null && echo "  Stopped hex-nexus (PID $NEXUS_PID)"
    [ -n "$CHAT_PID" ] && kill "$CHAT_PID" 2>/dev/null && echo "  Stopped hex-chat (PID $CHAT_PID)"
}
trap cleanup EXIT

check() {
    local label="$1"
    shift
    if "$@" >/dev/null 2>&1; then
        echo -e "  ${GREEN}✓${NC} $label"
        PASS=$((PASS + 1))
    else
        echo -e "  ${RED}✗${NC} $label"
        FAIL=$((FAIL + 1))
    fi
}

check_output() {
    local label="$1"
    local expected="$2"
    shift 2
    local output
    output=$("$@" 2>&1) || true
    if echo "$output" | grep -q "$expected"; then
        echo -e "  ${GREEN}✓${NC} $label"
        PASS=$((PASS + 1))
    else
        echo -e "  ${RED}✗${NC} $label (expected: $expected)"
        FAIL=$((FAIL + 1))
    fi
}

skip() {
    echo -e "  ${YELLOW}○${NC} $1 (skipped)"
    SKIP=$((SKIP + 1))
}

echo -e "${CYAN}══════════════════════════════════════════════════${NC}"
echo -e "${CYAN}  hex full-stack test suite${NC}"
echo -e "${CYAN}══════════════════════════════════════════════════${NC}"
echo ""

# ── Phase 1: Unit Tests ──────────────────────────────

echo -e "${CYAN}── Phase 1: Unit Tests ──${NC}"

check "hex-core compiles" cargo check -p hex-core
check "hex-core tests pass" cargo test -p hex-core --quiet
check "hex-agent compiles" cargo check -p hex-agent
check "hex-agent tests pass" cargo test -p hex-agent --quiet
check "hex-nexus lib compiles" cargo check -p hex-nexus
check "hex-nexus lib tests pass" cargo test -p hex-nexus --lib --quiet
check "hex-chat compiles" cargo check -p hex-chat
check "hex-cli compiles" cargo check -p hex-cli

echo ""
echo -e "${CYAN}── Phase 2: SpacetimeDB Module Tests ──${NC}"

(cd spacetime-modules && {
    # ADR-2604050900: right-sized to 7 modules
    check "hexflo-coordination tests" cargo test -p hexflo-coordination --quiet
    check "agent-registry tests" cargo test -p agent-registry --quiet
    check "inference-gateway tests" cargo test -p inference-gateway --quiet
    check "secret-grant tests" cargo test -p secret-grant --quiet
    check "rl-engine tests" cargo test -p rl-engine --quiet
    check "chat-relay tests" cargo test -p chat-relay --quiet
    check "neural-lab tests" cargo test -p neural-lab --quiet
})

echo ""
echo -e "${CYAN}── Phase 3: Architecture Health ──${NC}"

check_output "hex analyze grade A" "Grade:    A" hex analyze .
check_output "zero boundary violations" "Boundary violations    | 0" hex analyze .
check_output "zero circular deps" "Circular dependencies  | 0" hex analyze .

echo ""
echo -e "${CYAN}── Phase 4: Binary Builds ──${NC}"

check "hex-nexus builds (release)" cargo build --release -p hex-nexus
check "hex-agent builds (release)" cargo build --release -p hex-agent
check "hex-chat builds (release)" cargo build --release -p hex-chat
check "hex-cli builds (release)" cargo build --release -p hex-cli

# Quick mode stops here
if [ "${1:-}" = "--quick" ]; then
    echo ""
    echo -e "${CYAN}── Quick mode: skipping service tests ──${NC}"
    skip "hex-nexus daemon start"
    skip "hex-nexus health check"
    skip "hex-nexus API endpoints"
    skip "hex-chat web dashboard"
    skip "hex-cli MCP server"
else
    echo ""
    echo -e "${CYAN}── Phase 5: Service Startup ──${NC}"

    # Start hex-nexus
    ./target/release/hex-nexus --port $NEXUS_PORT --bind 127.0.0.1 &
    NEXUS_PID=$!
    sleep 2

    if kill -0 "$NEXUS_PID" 2>/dev/null; then
        echo -e "  ${GREEN}✓${NC} hex-nexus started (PID $NEXUS_PID, port $NEXUS_PORT)"
        PASS=$((PASS + 1))
    else
        echo -e "  ${RED}✗${NC} hex-nexus failed to start"
        FAIL=$((FAIL + 1))
        NEXUS_PID=""
    fi

    # Health check
    if [ -n "$NEXUS_PID" ]; then
        check_output "hex-nexus /api/version responds" "version" \
            curl -sf "http://127.0.0.1:$NEXUS_PORT/api/version"

        check_output "hex-nexus /api/swarms responds" "[" \
            curl -sf "http://127.0.0.1:$NEXUS_PORT/api/swarms"

        check_output "hex-nexus /api/agents responds" "[" \
            curl -sf "http://127.0.0.1:$NEXUS_PORT/api/agents"
    fi

    # Start hex-chat web
    ./target/release/hex-chat web --port $CHAT_PORT --nexus-url "http://127.0.0.1:$NEXUS_PORT" &
    CHAT_PID=$!
    sleep 2

    if kill -0 "$CHAT_PID" 2>/dev/null; then
        echo -e "  ${GREEN}✓${NC} hex-chat web started (PID $CHAT_PID, port $CHAT_PORT)"
        PASS=$((PASS + 1))
    else
        echo -e "  ${RED}✗${NC} hex-chat web failed to start"
        FAIL=$((FAIL + 1))
        CHAT_PID=""
    fi

    if [ -n "$CHAT_PID" ]; then
        check_output "hex-chat dashboard serves HTML" "hex-chat" \
            curl -sf "http://127.0.0.1:$CHAT_PORT/"
    fi

    echo ""
    echo -e "${CYAN}── Phase 6: MCP Server ──${NC}"

    check_output "hex-cli MCP responds to initialize" "serverInfo" \
        bash -c 'echo "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}" | timeout 3 ./target/release/hex mcp 2>/dev/null || true'

    echo ""
    echo -e "${CYAN}── Phase 7: Integration ──${NC}"

    if [ -n "$NEXUS_PID" ]; then
        # Create a swarm via API
        SWARM_RESP=$(curl -sf -X POST "http://127.0.0.1:$NEXUS_PORT/api/swarms" \
            -H 'Content-Type: application/json' \
            -d '{"name":"test-swarm","topology":"mesh"}' 2>/dev/null || echo "")

        if echo "$SWARM_RESP" | grep -q "id"; then
            echo -e "  ${GREEN}✓${NC} Swarm creation via API"
            PASS=$((PASS + 1))

            SWARM_ID=$(echo "$SWARM_RESP" | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4)

            if [ -n "$SWARM_ID" ]; then
                check_output "Task creation via API" "id" \
                    curl -sf -X POST "http://127.0.0.1:$NEXUS_PORT/api/swarms/$SWARM_ID/tasks" \
                    -H 'Content-Type: application/json' \
                    -d '{"title":"test-task"}'
            fi
        else
            echo -e "  ${RED}✗${NC} Swarm creation via API"
            FAIL=$((FAIL + 1))
        fi

        # HexFlo memory store/retrieve
        curl -sf -X POST "http://127.0.0.1:$NEXUS_PORT/api/hexflo/memory" \
            -H 'Content-Type: application/json' \
            -d '{"key":"test-key","value":"test-value"}' >/dev/null 2>&1 || true

        check_output "HexFlo memory store/retrieve" "test-value" \
            curl -sf "http://127.0.0.1:$NEXUS_PORT/api/hexflo/memory/test-key"
    fi
fi

# ── Summary ──────────────────────────────────────────

echo ""
echo -e "${CYAN}══════════════════════════════════════════════════${NC}"
TOTAL=$((PASS + FAIL + SKIP))
if [ $FAIL -eq 0 ]; then
    echo -e "  ${GREEN}ALL PASS${NC}: $PASS passed, $SKIP skipped, $FAIL failed (of $TOTAL)"
    echo -e "${CYAN}══════════════════════════════════════════════════${NC}"
    exit 0
else
    echo -e "  ${RED}FAILURES${NC}: $PASS passed, $SKIP skipped, $FAIL failed (of $TOTAL)"
    echo -e "${CYAN}══════════════════════════════════════════════════${NC}"
    exit 1
fi
