# sandbox-demo

A simple Rust key-value store REST API built by a `hex-agent` running inside a **Docker AI Sandbox** microVM.

This example demonstrates the full ADR-2603282000 workflow:

```
hex swarm spawn → docker run hex-agent:latest → agent polls HexFlo task
  → writes files via hex_write_file (architecture-enforced)
  → verifies with hex_bash("cargo check && cargo test")
  → commits via hex_git_commit("feat(sandbox-demo): implement kv-store api")
  → reports completion to HexFlo
```

## Run the app

```bash
cargo run
# → sandbox-demo listening on http://0.0.0.0:3030
```

## API

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Health check |
| GET | `/entries` | List all entries |
| POST | `/entries` | Create entry `{key, value}` → 201 |
| GET | `/entries/:id` | Get entry by ID → 404 if missing |
| DELETE | `/entries/:id` | Delete entry → 204 |
| GET | `/entries/by-key/:key` | Look up by key name |

```bash
# Create
curl -X POST http://localhost:3030/entries \
  -H 'content-type: application/json' \
  -d '{"key":"greeting","value":"hello from sandbox"}'

# List
curl http://localhost:3030/entries

# Get by key
curl http://localhost:3030/entries/by-key/greeting
```

## Tests

```bash
cargo test
```

## Build this via Docker sandbox

### 1. Build the hex-agent image

```bash
docker build -t hex-agent:latest hex-agent/
```

### 2. Start hex-nexus

```bash
hex nexus start
```

### 3. Submit the task to HexFlo

```bash
hex task create sandbox-swarm "$(cat examples/sandbox-demo/hex-task.json | jq -r .title)"
```

Or use the REST API:

```bash
curl -X POST http://localhost:5555/api/hexflo/tasks \
  -H 'content-type: application/json' \
  -d @examples/sandbox-demo/hex-task.json
```

### 4. Spawn an agent in the sandbox

hex-nexus automatically uses `docker run` when Docker is available and `hex-agent:latest` exists:

```bash
curl -X POST http://localhost:5555/api/agents/spawn \
  -H 'content-type: application/json' \
  -d '{
    "projectDir": "'$(pwd)'",
    "worktreeBranch": "feat/sandbox-demo/api",
    "secretKeys": ["ANTHROPIC_API_KEY"]
  }'
```

The agent will:
1. Pick up the task via `GET /api/hexflo/tasks/claim`
2. Run `hex dev start --auto "Build a simple Rust key-value store REST API"`
3. Write and test the code using hex MCP tools (all file writes are architecture-enforced)
4. Commit and report completion

### What the agent cannot do inside the sandbox

- Write files outside `/workspace` (path traversal → rejected by `hex_write_file`)
- Import across adapter boundaries (`adapters/primary` ↔ `adapters/secondary`)
- Run disallowed shell commands (`curl`, `rm -rf`, `sudo`)
- Access network endpoints not in the sandbox allow-list

## Architecture notes

This app was built by the sandbox agent using only hex MCP tools — no Claude Code, no direct filesystem access. The agent ran in a microVM with:
- Only `/workspace` (this worktree) visible
- Network restricted to nexus:5555, SpacetimeDB:3033, Ollama:11434, openrouter.ai:443, crates.io:443
- Every file write validated for path safety and hex boundary compliance
