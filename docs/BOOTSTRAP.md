# hex bootstrap

Single command to orchestrate full hex environment setup: verify prerequisites, start services in parallel, load models, configure project, validate. Eliminates manual service juggling and unclear setup steps (ADR-2604170001).

## Quick Start

```bash
# Dry-run: see what would happen without side effects
hex bootstrap --dry-run

# Full setup with local Ollama (default)
hex bootstrap

# CI environment (skip OS checks, use Claude API)
hex bootstrap --profile ci --skip-prereq

# Production setup
hex bootstrap --profile prod
```

## Phases

### Phase 1: Prerequisites Check
Detects OS and checks for required tools. Suggests install commands if missing.

- **SpacetimeDB**: `brew install spacetimedb` (macOS) or curl installer (Linux)
- **Ollama**: `brew install ollama` (macOS) or curl installer (Linux)
- **Rust**: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- **Bun**: `curl -fsSL https://bun.sh/install | bash`
- **Cargo**: Part of Rust install

Skip with `--skip-prereq` (useful in CI).

### Phase 2: Service Startup
Starts three services in parallel with health checks:

1. **SpacetimeDB** (port 3033): Coordination & state core
2. **hex-nexus** (port 5555): Filesystem bridge daemon & dashboard
3. **Ollama** (port 11434): Local inference engine

Each service:
- Checks if already running (idempotent by default)
- Spawns only if not alive
- Validates health via port/socket check (30s timeout, 3 retries with backoff)
- Returns PID and status

Pass `--force` to restart running services.

### Phase 3: Model Loading
Pulls default inference models from Ollama:

- **T1** (scaffold/transform): `qwen3:4b` (4.7 GB)
- **T2** (standard codegen): `qwen2.5-coder:32b` (19 GB)
- **T2.5** (reasoning): `devstral-small-2:24b` (14 GB)

Models retry with exponential backoff on timeout. Skip model pulling with `--skip-models`.

### Phase 4: Configuration Setup
Creates/preserves `.hex/project.json` with default tier models and bootstrap metadata.

```json
{
  "inference": {
    "tier_models": {
      "t1": "qwen3:4b",
      "t2": "qwen2.5-coder:32b",
      "t2_5": "devstral-small-2:24b"
    }
  },
  "bootstrap": {
    "profile": "dev",
    "timestamp": "2026-04-17T...",
    "services_started": ["spacetimedb", "hex-nexus", "ollama"]
  }
}
```

### Phase 5: Validation
Runs health checks and displays formatted bootstrap status table.

## Profiles

| Profile | Use Case | Inference | Notes |
|---------|----------|-----------|-------|
| **dev** | Local development | Ollama (local) | Default. Models cached in `~/.ollama/` |
| **ci** | GitHub Actions | Claude API | Skips OS checks. Requires `ANTHROPIC_API_KEY` |
| **prod** | Remote deployment | Managed service | Requires network access & credentials |

Select with `--profile <dev|ci|prod>`.

## Flags

```
--profile <PROFILE>    dev (default), ci, or prod
--dry-run              Show what would happen, no side effects
--skip-models          Skip model downloading (T1/T2/T2.5)
--skip-prereq          Skip OS prerequisite checks (for CI)
--force                Restart services even if already running
--verbose              Enable debug logging
```

## Common Workflows

### Local Development Setup
```bash
hex bootstrap
```

Starts fresh with all defaults: checks prerequisites, starts all services, loads models, creates config.

### CI Pipeline
```bash
hex bootstrap --profile ci --skip-prereq --skip-models
```

Assumes container already has Rust/Bun. Skips OS checks. Loads models from cache or on-demand.

### Idempotent Setup (repeated runs)
```bash
hex bootstrap
```

By default, skips services that are already running. Models are reused if already loaded. Config is preserved. Safe to run multiple times.

### Force Restart (troubleshooting)
```bash
hex bootstrap --force
```

Kills and restarts all services. Useful if services are hung or in a bad state.

## Troubleshooting

### "Prerequisites check failed: SpacetimeDB not found"

**Solution**: Install via suggested command in output.

```bash
# macOS
brew install spacetimedb

# Linux
curl --proto '=https' --tlsv1.2 -sSf https://install.spacetimedb.com | sh
```

### "hex-nexus failed to start (port 5555 unreachable)"

**Causes & fixes**:

1. **Service died**: Check logs
   ```bash
   hex nexus status
   ```

2. **Port in use**: Kill conflicting process
   ```bash
   lsof -i :5555
   pkill -f hex-nexus
   hex bootstrap --force
   ```

3. **Dependencies missing**: Ensure SpacetimeDB is running
   ```bash
   spacetime server status
   ```

### "Ollama model pull timeout"

**Causes & fixes**:

1. **Network timeout**: Increase patience, retries are automatic
   ```bash
   # Wait 5 minutes for T2 (19 GB) on slow connection
   hex bootstrap --skip-models  # Skip, then
   ollama pull qwen2.5-coder:32b  # Manual pull
   ```

2. **Disk full**: Free space
   ```bash
   df -h ~/.ollama/
   ```

3. **Ollama hung**: Restart and retry
   ```bash
   hex bootstrap --force
   ```

### "Validation failed: config (.hex/project.json) missing"

**Solution**: Config is created in Phase 4. If missing:

```bash
hex bootstrap --skip-prereq --skip-models  # Skip slow phases
```

Or manually:

```bash
mkdir -p .hex
cat > .hex/project.json <<'EOF'
{
  "inference": {
    "tier_models": {
      "t1": "qwen3:4b",
      "t2": "qwen2.5-coder:32b",
      "t2_5": "devstral-small-2:24b"
    }
  }
}
EOF
```

### "Validation shows models not loaded"

**Context**: Models are optional for bootstrap success. They load on first inference request if missing.

**To load now**:

```bash
hex bootstrap --skip-prereq --skip-models  # Skip other phases
# Then manually:
ollama pull qwen3:4b
ollama pull qwen2.5-coder:32b
ollama pull devstral-small-2:24b
```

## Next Steps

After successful bootstrap:

```bash
# View dashboard
open http://localhost:5555

# Execute first workplan
hex plan list
hex plan execute <workplan.json>

# Check system health
hex status
hex analyze .
```

## Related

- **ADR-2604170001**: Bootstrap design & phases
- **ADR-2604120202**: Tiered inference routing (T1/T2/T2.5)
- **ADR-2604131800**: Self-hosting gaps (bootstrap is Phase 1 fix)
