#!/usr/bin/env bash
# claude-on-hex — launch Claude Code against hex's Anthropic-compatible gateway
# (/v1/messages) instead of Anthropic's cloud. Every request then flows through
# hex's tiered, local-first, circuit-broken routing (ADR-2026-07-10-1000), so
# Claude Code runs on local Ollama models with cloud as a fallback.
#
# Usage:
#   scripts/claude-on-hex.sh [any claude args...]
#   scripts/claude-on-hex.sh "explain this repo"
#
# Env overrides:
#   HEX_NEXUS_HOST   nexus host         (default: 127.0.0.1)
#   HEX_NEXUS_PORT   nexus port         (default: 5555)
#   HEX_CLAUDE_MODEL model id to pin    (default: unset → hex picks per tier;
#                                        use "hex/<provider-id>" to pin one)
#
# Dev utility only (per the "No Runtime Scripts" rule): this just wires env vars
# and execs the claude CLI. All actual inference routing lives in hex-nexus.

set -euo pipefail

HEX_NEXUS_HOST="${HEX_NEXUS_HOST:-127.0.0.1}"
HEX_NEXUS_PORT="${HEX_NEXUS_PORT:-5555}"
BASE_URL="http://${HEX_NEXUS_HOST}:${HEX_NEXUS_PORT}"

# 1. claude CLI present?
if ! command -v claude >/dev/null 2>&1; then
  echo "✗ 'claude' (Claude Code) is not on PATH. Install it first:" >&2
  echo "    npm install -g @anthropic-ai/claude-code" >&2
  exit 1
fi

# 2. hex-nexus reachable? Probe the gateway's models list (cheap, no inference).
if ! curl -sf -o /dev/null --max-time 5 "${BASE_URL}/v1/models"; then
  echo "✗ hex-nexus gateway not reachable at ${BASE_URL}/v1/models" >&2
  echo "  Start it with:  hex nexus start" >&2
  echo "  (or set HEX_NEXUS_HOST / HEX_NEXUS_PORT if it runs elsewhere)" >&2
  exit 1
fi

# 3. Point Claude Code at hex. ANTHROPIC_AUTH_TOKEN just needs to be non-empty;
#    hex does not check it. API_KEY is cleared so Claude Code doesn't try the
#    real Anthropic endpoint. The 64k context floor is Claude Code's documented
#    minimum for local-model setups.
export ANTHROPIC_BASE_URL="${BASE_URL}"
export ANTHROPIC_AUTH_TOKEN="hex"
export ANTHROPIC_API_KEY=""
export CLAUDE_CODE_MAX_OUTPUT_TOKENS="${CLAUDE_CODE_MAX_OUTPUT_TOKENS:-8192}"

echo "⬡ Claude Code → hex gateway (${BASE_URL}/v1/messages) → local-first routing"

# 4. --auto shortcut: if the first arg is "--auto" (or "auto"), enable
#    auto-accept-edits and default to a stronger local coder, since autonomous
#    loops on a weak model drift. You still gate shell commands; use
#    --dangerously-skip-permissions yourself for full bypass. Strip the token
#    and inject the flags; everything after passes through.
EXTRA_ARGS=()
if [[ "${1:-}" == "--auto" || "${1:-}" == "auto" ]]; then
  shift
  EXTRA_ARGS+=(--permission-mode acceptEdits)
  : "${HEX_CLAUDE_MODEL:=hex/qwen2.5-coder:32b}"
  echo "  auto mode: --permission-mode acceptEdits (shell commands still gated)"
fi

# 5. Optional model pin. With none, hex routes by tier (recommended).
if [[ -n "${HEX_CLAUDE_MODEL:-}" ]]; then
  echo "  pinned model: ${HEX_CLAUDE_MODEL}"
  exec claude --model "${HEX_CLAUDE_MODEL}" "${EXTRA_ARGS[@]}" "$@"
fi

exec claude "${EXTRA_ARGS[@]}" "$@"
