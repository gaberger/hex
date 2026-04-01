Create and manage a structured task list for the current coding session.

Use for:
1. Complex multi-step tasks requiring 3+ distinct steps
2. Tasks requiring careful planning before execution
3. When the user provides multiple tasks at once

States: pending, in_progress, completed. Update status in real-time. Mark tasks complete IMMEDIATELY after finishing, not at the end of all work.

## hex-specific rules

### TodoWrite vs HexFlo tasks

These are complementary, not alternatives:

| Tool | Purpose |
|---|---|
| TodoWrite | In-session task tracking — visible in the UI, ephemeral |
| `mcp__hex__hex_hexflo_task_create` | Persistent cross-session swarm task tracking in SpacetimeDB |

For workplan execution: use **both** — TodoWrite for your own step-by-step progress, HexFlo tasks for swarm-level coordination and cross-agent visibility.

### When executing a workplan

Create a TodoWrite entry for each workplan phase as you begin it:
1. Mark the phase `in_progress` before starting
2. Mark it `completed` immediately after the phase is done and verified
3. Never batch-complete at the end — real-time status matters for swarm coordination

### Task granularity

Good task entries in hex context:
- "Implement PromptPort trait in hex-agent/src/ports/prompt.rs"
- "Add tool template for Bash in hex-cli/assets/context-templates/tools/"
- "Run cargo check -p hex-agent after port changes"

Too vague:
- "Work on context engineering"
- "Fix the thing"
