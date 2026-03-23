# hex Development Workflow Guide

This guide walks through the full hex development pipeline: from writing an ADR to shipping code via swarm-coordinated agents. It covers both the CLI commands and the AI agent integration (Claude Code / opencode).

## The Pipeline

hex enforces a strict development pipeline:

```
ADR → Workplan → Swarm → Tasks → Agents → Code → Validate
```

Each step must complete before the next can begin. In mandatory enforcement mode, hooks will block agents that skip steps.

## Phase 1: Architecture Decision Record (ADR)

Every non-trivial change starts with an ADR documenting the decision and rationale.

### Create an ADR

Use the `/hex-adr-create` skill in Claude Code, or create the file manually:

```bash
# Check next available ADR number
hex adr schema

# Create the file (timestamp-based ID: YYMMDDHHMM)
# Example: docs/adrs/ADR-2603241430-my-feature.md
```

ADR template:

```markdown
# ADR-YYMMDDHHMM: Feature Title

**Status:** Proposed
**Date:** YYYY-MM-DD
**Drivers:** Why this change is needed

## Context
What's the current situation and what forces are at play?

## Decision
What are we going to do?

## Consequences
### Positive
### Negative
### Mitigations
```

### Manage ADRs

```bash
hex adr list                    # List all ADRs with status
hex adr status ADR-2603241430   # Show detail for one ADR
hex adr search "openrouter"     # Search by keyword
hex adr abandoned               # Find stale/abandoned ADRs
hex adr review                  # Check consistency issues
```

## Phase 2: Workplan

A workplan decomposes the ADR into adapter-bounded steps organized by dependency tier.

### Create a Workplan

```bash
# Reference the ADR (required unless --no-adr)
hex plan create "implement OpenRouter inference integration" --adr ADR-2603231600

# Specify target language
hex plan create "add caching layer" --adr ADR-050 --lang rust

# Without ADR (escape hatch for quick fixes)
hex plan create "fix typo in dashboard" --no-adr
```

This generates a JSON file in `docs/workplans/` with steps organized by tier:

| Tier | Layer | Depends On |
|------|-------|------------|
| 0 | Domain + Ports | Nothing |
| 1 | Secondary adapters | Tier 0 |
| 2 | Primary adapters | Tier 0 |
| 3 | Use cases | Tiers 0-2 |
| 4 | Composition root | Tiers 0-3 |
| 5 | Integration tests | Everything |

### Manage Workplans

```bash
hex plan list                          # List all workplans
hex plan status feat-my-feature.json   # Show steps and progress
hex plan active                        # Show running/paused plans
hex plan history                       # Show past executions
hex plan report <execution-id>         # Aggregate report
hex plan schema                        # Show the JSON schema
```

## Phase 3: Swarm

A swarm is a named coordination container that tracks tasks, agents, and progress.

### Initialize a Swarm

```bash
# Basic (hierarchical topology — one coordinator, many workers)
hex swarm init my-feature

# Pipeline (sequential steps)
hex swarm init my-feature -t pipeline

# Mesh (peer-to-peer, for independent parallel work)
hex swarm init my-feature -t mesh
```

### Monitor Swarms

```bash
hex swarm status    # Show active swarms with task counts
hex swarm list      # List all swarms (active + completed)
```

## Phase 4: Tasks

Tasks map 1:1 to workplan steps. Each task is tracked in HexFlo (SpacetimeDB-backed).

### Create Tasks from Workplan Steps

```bash
# Get the swarm ID from swarm init output
hex task create <swarm-id> "step-1: Add OpenRouter provider to WASM module"
hex task create <swarm-id> "step-2: Extend OpenAiCompatAdapter"
hex task create <swarm-id> "step-3: Add model discovery CLI command"
```

### Manage Tasks

```bash
hex task list                              # List all tasks across swarms
hex task assign <task-id> <agent-id>       # Assign to an agent
hex task complete <task-id> "result summary"  # Mark done
```

## Phase 5: Agent Execution

This is where inference happens. Agents (Claude Code, opencode, or hex-agent) execute the workplan steps, using registered inference providers (including OpenRouter) to generate code.

### From Claude Code (MCP tools)

Claude Code agents use hex MCP tools. The typical flow:

```
1. /hex-feature-dev          — Interactive feature development skill
2. Agent spawns subagents    — Each gets a HEXFLO_TASK:{id} in their prompt
3. Hooks auto-track          — pre-agent/post-agent hooks sync task status
4. hex analyze .             — Validate architecture after code changes
```

### From opencode

opencode agents get hex context injected automatically (ADR-2603231800):

```bash
hex opencode inject    # Inject hex context into opencode config
```

### From hex-agent (standalone)

hex-agent is the standalone runtime that can call inference directly:

```bash
# hex-agent uses registered providers from hex inference list
# OpenRouter models are available after: hex inference discover --provider openrouter
```

### How OpenRouter Models Get Used

When an agent makes an inference call, the routing works like this:

```
Agent request → hex-nexus /api/chat/completions
             → Checks provider for the requested model
             → If openrouter-*: adds X-Title, HTTP-Referer headers
             → Forwards to https://openrouter.ai/api/v1/chat/completions
             → Extracts actual cost from response
             → Reports cost to SpacetimeDB for budget tracking
```

The RL model selection engine (ADR-031) can automatically choose between providers based on task type, balancing cost and quality.

## Phase 6: Validate

After code is written, validate architecture compliance:

```bash
hex analyze .    # Full architecture health check
hex test         # Run integration tests
```

The validation judge (`/hex-validate` skill) performs semantic validation against the behavioral specs.

## Phase 7: Ship

```bash
hex status       # Project overview
hex git status   # Git state
hex git diff     # Review changes
```

Then commit via your preferred method or `/commit` skill.

## Complete Example

Here's a full workflow for adding a caching layer:

```bash
# 1. ADR — document the decision
#    Create docs/adrs/ADR-2603241430-response-caching.md

# 2. Workplan — decompose into steps
hex plan create "add response caching to inference endpoints" \
  --adr ADR-2603241430 --lang rust

# 3. Swarm — coordinate agents
hex swarm init response-caching
# → Returns swarm ID: abc123...

# 4. Tasks — register work items
hex task create abc123 "step-1: Add CachePort trait to ports/"
hex task create abc123 "step-2: Implement Redis adapter"
hex task create abc123 "step-3: Wire into inference route"
hex task create abc123 "step-4: Integration tests"

# 5. Execute — agents do the work
#    In Claude Code: /hex-feature-dev
#    Or spawn background agents with HEXFLO_TASK:{task-id}

# 6. Validate
hex analyze .
hex test

# 7. Ship
hex status
```

## Enforcement Modes

hex hooks enforce the pipeline. Check current mode:

```bash
hex enforce mode    # Show current enforcement level
hex enforce list    # Show all enforcement rules
```

| Mode | Behavior |
|------|----------|
| `advisory` | Warns when steps are skipped (default) |
| `mandatory` | Blocks agents that skip ADR/workplan/swarm steps |

In mandatory mode, a background agent spawned without an active swarm gets blocked:

```
⛔ Background agent blocked — no active workplan (ADR-2603221939)
  Pipeline: ADR → Workplan → Swarm → Agent
  Create a workplan first: hex plan create <requirements> --adr <ADR-ID>
```

## Inference Provider Selection

OpenRouter models participate alongside other providers. The system selects based on task type:

```bash
# See all available providers
hex inference list

# Test a specific model
hex inference test openrouter-meta-llama-llama-4-maverick

# See what the RL engine is selecting
# (visible in dashboard → Inference panel)
```

For setup, see [OpenRouter Setup Guide](./openrouter-setup.md).

## Quick Reference

```bash
# Pipeline commands
hex adr list                           # List ADRs
hex plan create "desc" --adr ADR-XXX   # Create workplan
hex swarm init my-feature              # Start swarm
hex task create <swarm> "step"         # Add task
hex task complete <task> "result"      # Mark done

# Monitoring
hex swarm status                       # Active swarms
hex task list                          # All tasks
hex plan active                        # Running plans
hex status                             # Project overview

# Validation
hex analyze .                          # Architecture check
hex enforce mode                       # Enforcement level

# Inference
hex inference list                     # Registered providers
hex inference discover --provider openrouter  # Sync models
hex inference test <provider-id>       # Test connectivity
```
