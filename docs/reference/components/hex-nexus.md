# Component: hex-nexus

## One-Line Summary

Filesystem bridge daemon — bridges SpacetimeDB's sandboxed WASM execution and the local operating system, providing REST API, architecture analysis, git operations, config sync, and dashboard serving on port 5555.

## Key Facts

- Rust binary (axum web framework), runs on port 5555
- 95+ REST API endpoints across 16 resource groups
- Serves hex-dashboard frontend (Solid.js SPA baked in via `rust-embed`)
- Syncs repo config files → SpacetimeDB tables on startup (ADR-044)
- Primary state: SpacetimeDB; fallback: SQLite (`~/.hex/hub.db`)
- HexFlo coordination module for native swarm orchestration (ADR-027)
- Requires SpacetimeDB to be running for full functionality

## Why It Exists

SpacetimeDB WASM modules cannot:
- Access the filesystem
- Spawn processes
- Make network calls (HTTP, SSH)
- Execute shell commands

hex-nexus performs all of these operations on behalf of the system. It is the explicit boundary between SpacetimeDB's pure transactional state and the side-effect world of operating system interaction.

## Source Structure

```
hex-nexus/
├── src/
│   ├── lib.rs              # HubConfig, build_app() — assembles axum router
│   ├── bin/hex-nexus.rs    # Daemon binary entry point
│   ├── routes/mod.rs       # All 95+ route registrations
│   ├── analysis/           # Architecture analysis
│   │   ├── analyzer.rs     # Main analysis orchestrator
│   │   ├── boundary_checker.rs
│   │   ├── cycle_detector.rs
│   │   ├── dead_export_finder.rs
│   │   ├── treesitter_adapter.rs  # TS/Go/Rust parsing
│   │   ├── adr_compliance.rs
│   │   └── layer_classifier.rs
│   ├── coordination/       # HexFlo (ADR-027)
│   │   ├── mod.rs          # HexFlo struct — swarm/task/agent API
│   │   ├── memory.rs       # Scoped key-value store
│   │   └── cleanup.rs      # Heartbeat timeout + task reclamation
│   ├── adapters/           # State backend adapters
│   │   ├── spacetime_state.rs  # SpacetimeDB adapter (HTTP reducer calls)
│   │   └── sqlite_state.rs    # SQLite fallback
│   ├── config_sync.rs      # Repo → SpacetimeDB config sync
│   ├── git/                # Git introspection
│   │   ├── blame.rs, diff.rs, log.rs
│   │   └── worktree.rs     # Worktree management
│   ├── orchestration/      # Agent/workplan management
│   │   ├── agent_manager.rs
│   │   ├── constraint_enforcer.rs
│   │   └── workplan_executor.rs
│   ├── ports/
│   │   └── state.rs        # IStatePort trait (dual backend)
│   └── middleware/
│       ├── auth.rs         # Bearer token authentication
│       └── deprecation.rs  # X-Deprecated headers
├── assets/                 # Dashboard frontend (Solid.js)
│   ├── index.html
│   ├── package.json        # Vite, Solid.js, TailwindCSS, SpacetimeDB SDK
│   └── src/
│       ├── app/App.tsx     # Main component
│       ├── components/     # ControlPlane, AgentFleet, ProjectDetail, etc.
│       ├── hooks/          # Reactive hooks
│       ├── spacetimedb/    # Auto-generated client bindings
│       └── stores/         # connection, router, ui, chat, hexflo-monitor
└── Cargo.toml              # axum 0.8, tokio, spacetimedb-sdk 2.0 (optional)
```

## REST API Surface

### Project Management
| Method | Path | Purpose |
|:-------|:-----|:--------|
| GET | `/api/projects` | List registered projects |
| POST | `/api/projects/register` | Register a project |
| POST | `/api/projects/init` | Initialize a project |
| DELETE | `/api/projects/{id}` | Unregister project |

### Architecture Analysis
| Method | Path | Purpose |
|:-------|:-----|:--------|
| POST | `/api/analyze` | Analyze a path |
| GET | `/api/{project_id}/analyze` | Analyze project (JSON) |
| GET | `/api/{project_id}/analyze/text` | Analyze project (text) |
| POST | `/api/analyze/adr-compliance` | Check ADR compliance |

### Swarm Coordination
| Method | Path | Purpose |
|:-------|:-----|:--------|
| POST | `/api/swarms` | Create swarm |
| GET | `/api/swarms/active` | List active swarms |
| GET | `/api/swarms/{id}` | Get swarm details |
| POST | `/api/swarms/{id}/tasks` | Create task |
| PATCH | `/api/swarms/{id}/tasks/{task_id}` | Update task |

### Multi-Instance Coordination
| Method | Path | Purpose |
|:-------|:-----|:--------|
| POST | `/api/coordination/instance/register` | Register instance |
| POST | `/api/coordination/instance/heartbeat` | Instance heartbeat |
| POST | `/api/coordination/worktree/lock` | Acquire worktree lock |
| POST | `/api/coordination/task/claim` | Claim task |
| POST | `/api/coordination/cleanup` | Cleanup stale sessions |

### Git Integration (ADR-044)
| Method | Path | Purpose |
|:-------|:-----|:--------|
| GET | `/api/{project_id}/git/status` | Git status |
| GET | `/api/{project_id}/git/log` | Git log |
| GET | `/api/{project_id}/git/diff` | Git diff |
| GET | `/api/{project_id}/git/branches` | List branches |
| GET | `/api/{project_id}/git/worktrees` | List worktrees |
| POST | `/api/{project_id}/git/worktrees` | Create worktree |
| DELETE | `/api/{project_id}/git/worktrees/{name}` | Delete worktree |

### Inference
| Method | Path | Purpose |
|:-------|:-----|:--------|
| POST | `/api/inference/register` | Register provider |
| POST | `/api/inference/complete` | Request completion |
| GET | `/api/inference/endpoints` | List providers |

### HexFlo Memory
| Method | Path | Purpose |
|:-------|:-----|:--------|
| POST | `/api/hexflo/memory` | Store memory |
| GET | `/api/hexflo/memory/{key}` | Retrieve memory |
| GET | `/api/hexflo/memory/search` | Search memory |

### WebSocket
| Path | Purpose |
|:-----|:--------|
| `/ws` | Main real-time event stream |
| `/ws/chat` | Chat-specific WebSocket |

*(See full API: `GET /api/openapi.json` or `GET /api/docs`)*

## Configuration

**Start/stop:**
```bash
hex nexus start      # Start daemon
hex nexus stop       # Stop daemon
hex nexus status     # Check health
```

**State backend:** `.hex/state.json`
```json
{
  "backend": "spacetimedb",
  "spacetimedb": { "host": "localhost:3000", "database": "hex-nexus" }
}
```

**Dashboard assets:** Editing any file in `hex-nexus/assets/` requires rebuild:
```bash
cd hex-nexus && cargo build --release
# Then restart daemon and hard-refresh browser (Cmd+Shift+R)
```

**Cargo features:**
```toml
[features]
default = ["spacetimedb", "sqlite-session"]
spacetimedb = ["spacetimedb-sdk"]  # SpacetimeDB state adapter
sqlite-session = []                 # Chat session persistence
```

## Depends On

- **SpacetimeDB** — state backend, reducer calls via HTTP API
- **hex-core** — shared domain types and port traits

## Depended On By

- **hex-cli** — delegates all commands to hex-nexus REST API
- **hex-dashboard** — served by hex-nexus (embedded assets)
- **hex-agent** — filesystem and git operations

## Related ADRs

- ADR-024: Hex-Hub Autonomous Nexus (origin)
- ADR-025: SpacetimeDB as State Backend
- ADR-027: HexFlo Swarm Coordination
- ADR-032: Deprecate hex-hub (migration to hex-nexus)
- ADR-034: Migrate Analyzer to Rust
- ADR-044: Config Sync to SpacetimeDB
