# Remote Agent Walkthrough — From Task to Code on a GPU Box

This document walks through every step of hex's distributed agent execution:
a coordinator on your Mac creates a task, a worker on a remote GPU box picks it up,
generates code using local Ollama, and reports the result back. All via SSH tunnel,
zero cloud APIs, $0 cost.

## Prerequisites

```
Mac (coordinator):      hex-nexus running, SpacetimeDB running
Bazzite (GPU worker):   hex-cli built, Ollama with qwen2.5-coder:32b
SSH:                    passwordless SSH from Mac → bazzite
```

## Step 1: Start the Coordinator

On your Mac:

```bash
hex nexus start --bind 0.0.0.0
```

This starts:
- **SpacetimeDB** on `:3033` (state backend)
- **hex-nexus** on `:5555` (REST API, dashboard, coordination)
- **hex-agent** (local agent registration)

## Step 2: Create an SSH Tunnel

The Mac's firewall blocks incoming connections. An SSH reverse tunnel lets
bazzite reach the coordinator's nexus without firewall changes:

```bash
ssh -f -N -R 5556:localhost:5555 bazzite
```

Now bazzite can reach the coordinator at `http://localhost:5556`.

## Step 3: Create a Swarm and Task

On the Mac, create a swarm and add a code generation task:

```bash
hex swarm init gcd-demo
hex task list  # Verify swarm created
```

Or via REST (requires the agent ID header):

```bash
AGENT_ID=$(cat ~/.hex/sessions/agent-*.json | jq -r '.agentId')

# Create swarm
curl -X POST http://localhost:5555/api/swarms \
  -H "Content-Type: application/json" \
  -H "X-Hex-Agent-Id: $AGENT_ID" \
  -d '{"name":"gcd-demo","topology":"hierarchical"}'

# Create task
curl -X POST http://localhost:5555/api/swarms/$SWARM_ID/tasks \
  -H "Content-Type: application/json" \
  -H "X-Hex-Agent-Id: $AGENT_ID" \
  -d '{"title":"hex-coder: Write fn gcd(a: u64, b: u64) -> u64. Include tests."}'
```

At this point, `hex task list` shows:

```
╭──────────┬─────────┬───────┬──────────────┬──────────────────────────────────────╮
│ Swarm    │ Status  │ Agent │ Task ID      │ Title                                │
├──────────┼─────────┼───────┼──────────────┼──────────────────────────────────────┤
│ gcd-demo │ pending │       │ 4d3ef423-9d… │ hex-coder: Write fn gcd...           │
╰──────────┴─────────┴───────┴──────────────┴──────────────────────────────────────╯
```

## Step 4: Start the Worker on Bazzite

SSH into bazzite and start the worker:

```bash
OLLAMA_HOST=http://localhost:11434 \
HEX_PROVIDER=ollama \
HEX_MODEL=qwen2.5-coder:32b \
HEX_NEXUS_URL=http://localhost:5556 \
hex agent worker --role hex-coder --poll-interval 3
```

The worker:

```
⬡ Worker started: hex-coder-bazzite.lan (agent: f33ce0e4)
  Role:     hex-coder
  Swarm:    any
  Poll:     3s
```

## Step 5: Worker Claims and Executes

The worker automatically:

1. **Polls** the coordinator for pending tasks matching its role
2. **Claims** the task atomically (CAS — no double-assignment)
3. **Selects model** via RL engine or user override
4. **Generates code** using local Ollama (qwen2.5-coder:32b)
5. **Runs compile + test gates** (ADR-005)
6. **Reports result** back to coordinator

```
[claim] attempting task 4d3ef423 for role hex-coder
[claim] ✓ claimed task 4d3ef423
→ Executing task: 4d3ef423 — hex-coder: Write fn gcd...
  INFO selected model qwen2.5-coder:32b source=user-override
  INFO code step complete tokens=2249 cost_usd=0.0 duration_ms=45751
✓ Task completed: 4d3ef423 (status synced)
```

## Step 6: Coordinator Sees the Result

Back on the Mac:

```bash
$ hex task list

╭──────────┬───────────┬────────────────┬──────────────┬──────────────────────────╮
│ Swarm    │ Status    │ Agent          │ Task ID      │ Title                    │
├──────────┼───────────┼────────────────┼──────────────┼──────────────────────────┤
│ gcd-demo │ completed │ f33ce0e4-8f53… │ 4d3ef423-9d… │ hex-coder: Write fn gcd… │
╰──────────┴───────────┴────────────────┴──────────────┴──────────────────────────╯
```

The task is `completed`, assigned to bazzite's agent `f33ce0e4`. The coordinator
has full visibility into what happened on the remote machine.

## What Happened Under the Hood

```
Mac (coordinator)                   SSH Tunnel              Bazzite (worker)
┌──────────────────────┐     ┌──────────────────┐     ┌──────────────────────┐
│ 1. hex swarm init    │     │                  │     │                      │
│ 2. Create task       │     │                  │     │                      │
│    (pending)         │     │                  │     │                      │
│                      │     │ :5556 → :5555    │     │ 3. hex agent worker  │
│                      │◄────│                  │────►│    polls /swarms     │
│ task claimed         │     │                  │     │ 4. [claim] ✓         │
│ (in_progress)        │     │                  │     │ 5. Ollama generate   │
│                      │     │                  │     │    qwen2.5-coder:32b │
│                      │     │                  │     │    2249 tokens, 46s  │
│                      │     │                  │     │ 6. Compile + test    │
│ task completed       │◄────│                  │────►│ 7. PATCH result back │
│ (completed)          │     │                  │     │    status synced     │
│                      │     │                  │     │                      │
│ 8. hex task list     │     │                  │     │ 9. Poll for next...  │
│    shows completed   │     │                  │     │                      │
└──────────────────────┘     └──────────────────┘     └──────────────────────┘
```

## Key Properties

| Property | How hex enforces it |
|:---------|:-------------------|
| **Atomic task claiming** | CAS (compare-and-swap) — two workers can't claim the same task |
| **Role-based routing** | Worker filters by `--role hex-coder`; only claims matching tasks |
| **Model selection** | RL engine picks optimal model, user can override with `HEX_MODEL` |
| **Local inference** | Worker calls Ollama on localhost — no cloud APIs, no network latency |
| **Status tracking** | Coordinator sees real-time status via `hex task list` |
| **Heartbeat** | Worker sends heartbeat every 30s; stale after 45s, dead after 120s |
| **No exposed ports** | SSH reverse tunnel — coordinator's firewall stays locked |

## Scaling

Add more workers by starting `hex agent worker` on additional machines:

```bash
# Machine 2: another GPU box
HEX_NEXUS_URL=http://coordinator:5556 \
HEX_MODEL=qwen3.5:27b \
hex agent worker --role hex-coder

# Machine 3: a CPU-only box for T1 tasks
HEX_NEXUS_URL=http://coordinator:5556 \
HEX_MODEL=qwen3:4b \
hex agent worker --role hex-coder
```

Tasks are claimed atomically — no double-assignment. Each worker uses its
local Ollama with whatever model is available. The coordinator's RL engine
learns which worker/model pairing performs best per task type.

## Environment Variables

| Variable | Purpose | Example |
|:---------|:--------|:--------|
| `HEX_NEXUS_URL` | Coordinator nexus URL (via tunnel) | `http://localhost:5556` |
| `OLLAMA_HOST` | Local Ollama URL | `http://localhost:11434` |
| `HEX_MODEL` | Force a specific model | `qwen2.5-coder:32b` |
| `HEX_PROVIDER` | Force inference provider | `ollama` |
| `HEX_OUTPUT_DIR` | Where to write generated files | `/home/gary/project/src` |
