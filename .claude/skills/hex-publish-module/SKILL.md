---
name: hex-publish-module
description: Publish a SpacetimeDB WASM module with the full pipeline — publish, regen bindings, bump schema version, rebuild, restart
user_invocable: true
trigger: "publish module", "spacetimedb publish", "publish spacetimedb", "regen module", "update module schema"
---

# SpacetimeDB Module Publish Pipeline

When a SpacetimeDB WASM module schema changes (fields added/removed, reducer signatures changed), follow this exact pipeline:

## Steps

1. **Publish the module** (from its directory):
```bash
cd spacetime-modules/<MODULE_NAME>
echo "y" | spacetime publish <MODULE_NAME> --clear-database
```

2. **Regenerate TypeScript bindings**:
```bash
cd /Volumes/ExtendedStorage/PARA/01-Projects/hex-intf
spacetime generate --lang typescript \
  --out-dir hex-nexus/assets/src/spacetimedb/<MODULE_NAME> \
  --module-path spacetime-modules/<MODULE_NAME>
```

3. **Bump SCHEMA_VERSION** in `hex-nexus/assets/src/stores/connection.ts`:
   - Find `const SCHEMA_VERSION = "N"` and increment N
   - This forces browser localStorage token clearing on next load

4. **Rebuild hex-nexus + hex-cli**:
```bash
cargo build -p hex-nexus -p hex-cli --release
```

5. **Restart nexus**:
```bash
cargo run -p hex-cli --release -- nexus stop
sleep 1
cargo run -p hex-cli --release -- nexus start
```

6. **Re-register project + agent** (data was cleared):
```bash
# Register project
target/release/hex dashboard register /Volumes/ExtendedStorage/PARA/01-Projects/hex-intf

# Register agent
CLAUDE_PROJECT_DIR="/Volumes/ExtendedStorage/PARA/01-Projects/hex-intf" \
CLAUDE_SESSION_ID="session-$(date +%s)" \
CLAUDE_MODEL="claude-opus-4-6" \
node .claude/helpers/agent-register.cjs register
```

## Available modules

| Module | Directory | Tables |
|--------|-----------|--------|
| hexflo-coordination | spacetime-modules/hexflo-coordination | swarm, swarm_task, swarm_agent, project, compute_node, etc |
| agent-registry | spacetime-modules/agent-registry | agent, agent_heartbeat, agent_cleanup_log |
| inference-gateway | spacetime-modules/inference-gateway | inference_provider, inference_request, inference_response |
| secret-grant | spacetime-modules/secret-grant | secret_grant, grant_audit |
| rl-engine | spacetime-modules/rl-engine | model_score, selection_event |
| chat-relay | spacetime-modules/chat-relay | chat_message, chat_channel |
| neural-lab | spacetime-modules/neural-lab | neural_pattern, experiment |

## Critical notes

- **Never publish from workspace root** — `uuid` compile error. Always `cd` into the module directory first.
- **Always bump SCHEMA_VERSION** — stale SDK tokens cause `DataView` deserialization crashes.
- **`--clear-database` destroys all data** — re-registration of project and agents is required.
- **Rebuild is required** — the Rust binary embeds the reducer call signatures; stale binary → "invalid arguments" errors.
