# ADR-2604170001: Unified hex Bootstrap Command

## Status
ACCEPTED

## Context

Current hex setup requires manual orchestration of multiple services and configurations:
- Verifying/installing SpacetimeDB, Ollama, hex-cli
- Starting hex-nexus, SpacetimeDB, Ollama daemons in correct order
- Loading inference models
- Configuring project.json tier models
- Running diagnostics to validate setup
- Unclear prerequisites and error messages

This friction delays actual development work and creates frustration for new users and returning sessions.

## Decision

Introduce **`hex bootstrap`** — a single idempotent command that orchestrates the full setup pipeline:

1. **Prereq check**: Verify/install SpacetimeDB, Ollama, rust, bun, cargo
2. **Parallel startup**: Launch SpacetimeDB, hex-nexus, Ollama concurrently with health checks
3. **Model loading**: Download and load default inference models (qwen3:4b, qwen2.5-coder:32b, etc.)
4. **Configuration**: Create/update `.hex/project.json` with tier defaults
5. **Validation**: Run `hex doctor` to verify all components
6. **Report**: Show status table and what's ready vs. pending

Command signature:
```bash
hex bootstrap [--profile <dev|ci|prod>] [--skip-models] [--force]
```

## Consequences

**Positive:**
- Single command replaces 5–10 manual steps
- Idempotent: safe to run multiple times (detect already-running services)
- Clear error messages with remediation (e.g., "Ollama not installed; run: brew install ollama")
- Profiles let teams standardize (dev = local Ollama, CI = Claude API, prod = remote)
- New users unblock faster; returning sessions validate state in seconds

**Negative:**
- Adds complexity to hex-cli (service orchestration, parallelism, retry logic)
- Must handle platform differences (macOS vs Linux vs Windows)
- SpacetimeDB/Ollama version mismatches could still block
- User expectations: bootstrap should "just work" or frustration compounds

## Implementation Notes

- Use `hex nexus start`, `hex sched daemon`, etc. (CLI commands) rather than spawning binaries directly
- Parallel execution via `tokio::spawn`; collect results with `tokio::join!`
- Health checks: poll `/api/health` endpoints until responsive (5s timeout, 10 retries)
- Model loading: delegate to Ollama CLI (`ollama pull <model>`) with progress output
- Profiles stored in `.hex/bootstrap.json` or `.hex/project.json` under `bootstrap.profile`
- Dry-run mode (`--dry-run`) shows what would be done without side effects

## Related

- ADR-2604112000 (Standalone mode / OllamaInferenceAdapter)
- ADR-2604150000 (Scheduler daemon / brain)
- `/hex doctor` (diagnostics subcommand)
