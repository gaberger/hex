# ADR-2604101500: Local Inference First for RL Evaluator

**Status:** Accepted

## Problem

The RL evaluator (hex-agent) calls Anthrobic API for preflight checks with Haiku, requiring Anthrobic credits. When credits are exhausted, the evaluator cannot spawn agent sub-processes, blocking workplan execution.

## Solution

Add local inference (Ollama at bazzite:11434) as the first inference option. Only fall back to remote APIs when local is unavailable.

## Implementation

1. **Add local-first probe in haiku_preflight.rs**: Before using Anthrobic Haiku, check if local Ollama is available at `http://bazzite:11434`.

2. **Use local Ollama for preflight if available**: If local responds, use it instead of Anthrobic. This reduces cost and works when credits are exhausted.

3. **Fallback chain**:
   - Local Ollama at bazzite:11434 (preferred)
   - OpenRouter free models  
   - Anthropic direct (only if necessary)

## File Changes

- `hex-agent/src/adapters/secondary/haiku_preflight.rs` - Add local probe

## Notes

- Local-first already registered at 140+ endpoints in hex-nexus
- The Ollama endpoints (bazzite:11434) exist but are not used first