---
name: hex-adr-create
description: Create a new Architecture Decision Record with auto-numbering and schema validation
trigger: /hex-adr-create
---

# Create New ADR

## Steps

1. Get the next available ADR number and schema by running:
   ```bash
   hex adr schema
   ```
   This returns the next number (atomically reserved in SpacetimeDB), the template, valid statuses, and required sections.

2. Ask the user for:
   - Title (required)
   - Brief context description
   - Drivers (what triggered this decision)

3. Create `docs/adrs/ADR-{NNN}-{kebab-slug}.md` using the template from `hex adr schema`

4. Fill in all required sections:
   - Title: `# ADR-{NNN}: {Title}`
   - Status: `**Status:** Proposed`
   - Date: today's date (YYYY-MM-DD)
   - Drivers: from user input
   - Context: describe the problem, forces, constraints, alternatives
   - Decision: clear imperative language ("We will...", "The system shall...")
   - Consequences: positive, negative, mitigations
   - Implementation: phased table with status
   - References: related ADRs, issues, documents

5. If a reserved placeholder exists (`ADR-{NNN}-reserved.md`), delete it after creating the real ADR

## Schema Reference

Valid statuses: `Proposed | Accepted | Deprecated | Superseded | Abandoned`

Required frontmatter:
- `**Status:**` — one of valid statuses
- `**Date:**` — YYYY-MM-DD
- `**Drivers:**` — what triggered this decision
- `**Supersedes:**` — (optional) ADR-NNN if replacing

Required sections: Context, Decision, Consequences, Implementation, References

## Multi-Agent Safety

The `hex adr schema` command reserves the ADR number atomically via `POST /api/adr/reserve`. This prevents two concurrent agents from creating ADRs with the same number. The reservation writes a placeholder file that is replaced when the actual ADR is written.

## Example

User: `/hex-adr-create`
Assistant: runs `hex adr schema` to get next number (e.g., ADR-060)
Assistant: "What architectural decision needs to be recorded?"
User: "We should use WebSockets instead of polling for real-time updates"
-> Creates `docs/adrs/ADR-060-websocket-realtime.md` with all sections filled
