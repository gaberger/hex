# ADR-067: Hex Installation & Pipeline Validation

**Status:** Accepted
**Date:** 2026-03-31
**Drivers:** No install/pipeline validation — users cannot verify that hex is correctly installed, and there is no standardized way to validate the build pipeline (build → test → analyze → validate) for a hex project.

## Context

When a developer installs hex or creates a new hex project, there is no post-install verification that:
1. The hex binary is executable and version-compatible
2. The project structure is correctly scaffolded
3. The build pipeline (build → test → analyze → validate) works end-to-end

### Current State

- `hex init` scaffolds project files but doesn't verify the scaffold is valid
- `hex analyze` checks architecture but requires the project to already exist
- No `hex doctor` or `hex install check` command
- No CI/CD validation workflow for hex projects

### Forces

1. **Consumer confidence** — Users need to trust their hex installation is working
2. **CI/CD integration** — Build pipelines need a standardized validation command
3. **Onboarding** — New developers should get immediate feedback on setup issues

### Alternatives Considered

1. **Ad-hoc validation** — Let users run individual commands (build, test, analyze) manually
2. **Dashboard-only** — Rely on the dashboard to show project health
3. **Scaffold-validator** — Existing agent (from ADR-055) validates project output

## Decision

We will implement **two validation layers** for hex installation and pipeline:

### 1. Installation Verification (`hex doctor`)

A new CLI command that verifies:
- Hex binary is installed and executable
- Version compatibility with installed assets (agents, skills, hooks)
- hex-nexus connectivity (if running)
- Project structure validity

```bash
hex doctor                    # Full diagnostics
hex doctor --verbose          # Detailed output
hex doctor --fix              # Attempt auto-fix where possible
```

### 2. Pipeline Validation (`hex validate pipeline`)

A new CLI command that runs the full build pipeline:
- `build` → Compile/project build
- `test` → Run test suite (discovered from package.json/Cargo.toml)
- `hex analyze` → Architecture boundary checks
- `hex validate` → Behavioral spec validation

```bash
hex validate pipeline              # Full pipeline
hex validate pipeline --skip test  # Skip test phase
hex validate pipeline --strict     # Fail on warnings
```

### Integration Points

| Component | Changes |
|-----------|---------|
| hex-cli | New `doctor` and `validate pipeline` commands |
| hex-nexus | Optional health check endpoint (for remote diagnostics) |
| Scaffold-validator | Integrate into `hex init` post-scaffold |
| ADR-055 (Unified Test Harness) | Pipeline runs harness, not just tests |

## Consequences

**Positive:**
- Immediate feedback on installation issues
- Standardized CI/CD validation workflow
- Better onboarding experience for new hex users

**Negative:**
- Additional CLI complexity
- Pipeline validation adds time to CI

**Mitigations:**
- `hex doctor` is fast (no network calls by default)
- `hex validate pipeline --parallel` runs stages concurrently where possible

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add `hex doctor` command to hex-cli | Done |
| P2 | Add `hex validate pipeline` command | Done |
| P3 | Integrate scaffold-validator into `hex init` | Pending |
| P4 | Add `--parallel` flag for pipeline | Done |

## References

- ADR-006 (Packaging) — npm distribution format
- ADR-055 (Unified Test Harness) — Test discovery and execution
- ADR-057 (Unified Test Harness) — E2E validation gaps
- ADR-043 (Project Manifest) — `.hex/project.yaml` scaffolded by `hex init`