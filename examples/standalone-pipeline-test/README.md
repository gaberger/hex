# Standalone Pipeline Smoke Test

End-to-end test for hex's tiered inference routing (ADR-2604120202).

Sends code generation tasks at each tier to the mapped local model via Ollama,
then validates output with string matching (T1) or `rustc` compile gates (T2/T2.5).

## Tier → Model Mapping

| Tier | Model | Best-of-N | Task Type |
|------|-------|-----------|-----------|
| T1   | qwen3:4b | 1 | Trivial edits (rename, typo fix) |
| T2   | qwen2.5-coder:32b | 3 | Single function + tests |
| T2.5 | qwen3.5:27b | 5 | Multi-function CLI app |

## Prerequisites

- `hex nexus start` (starts SpacetimeDB + nexus)
- Ollama reachable at `bazzite:11434` (or set `OLLAMA_HOST`)
- `rustc` installed (for compile gates)

## Usage

```bash
# Run all tiers
./run.sh

# Run a single tier
./run.sh --tier T1
./run.sh --tier T2
./run.sh --tier T2.5

# Verbose (show compile errors)
./run.sh --verbose
```

## What it tests

1. **T1 (qwen3:4b)**: Can the smallest model handle trivial edits?
   - Variable rename
   - Typo fix in comment

2. **T2 (qwen2.5-coder:32b)**: Can the code model generate compilable functions?
   - Fibonacci (iterative)
   - Palindrome checker with unit tests
   - Uses Best-of-3 + compile gate

3. **T2.5 (qwen3.5:27b)**: Can the agentic model handle multi-function programs?
   - CLI argument parser with error handling
   - Uses Best-of-5 + compile gate
