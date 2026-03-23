---
name: hex-adr-create
description: Create a new Architecture Decision Record with timestamp-based ID (YYMMDDHHMM)
trigger: /hex-adr-create
---

# Create New ADR

## Steps

1. Get the next available ADR ID and schema by running:
   ```bash
   hex adr schema
   ```
   This returns a timestamp-based ID (YYMMDDHHMM format), the template, valid statuses, and required sections.

2. Ask the user for:
   - Title (required)
   - Brief context description
   - Drivers (what triggered this decision)

3. Create `docs/adrs/ADR-{YYMMDDHHMM}-{kebab-slug}.md` using the template from `hex adr schema`

4. Fill in all required sections:
   - Title: `# ADR-{YYMMDDHHMM}: {Title}`
   - Status: `**Status:** Proposed`
   - Date: today's date (YYYY-MM-DD)
   - Drivers: from user input
   - Context: describe the problem, forces, constraints, alternatives
   - Decision: clear imperative language ("We will...", "The system shall...")
   - Consequences: positive, negative, mitigations
   - Implementation: phased table with status
   - References: related ADRs, issues, documents

## ID Format

ADR IDs use **YYMMDDHHMM** (timestamp) format — e.g., `ADR-2603221500` means 2026-03-22 at 15:00.
This eliminates race conditions from sequential numbering. No reservation needed.

Legacy ADRs (ADR-001 through ADR-066) keep their original sequential IDs.

## Schema Reference

Valid statuses: `Proposed | Accepted | Deprecated | Superseded | Abandoned`

Required frontmatter:
- `**Status:**` — one of valid statuses
- `**Date:**` — YYYY-MM-DD
- `**Drivers:**` — what triggered this decision
- `**Supersedes:**` — (optional) ADR-YYMMDDHHMM if replacing

Required sections: Context, Decision, Consequences, Implementation, References

## Example

User: `/hex-adr-create`
Assistant: runs `hex adr schema` to get timestamp ID (e.g., ADR-2603221500)
Assistant: "What architectural decision needs to be recorded?"
User: "We should use WebSockets instead of polling for real-time updates"
-> Creates `docs/adrs/ADR-2603221500-websocket-realtime.md` with all sections filled
