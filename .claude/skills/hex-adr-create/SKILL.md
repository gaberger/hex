---
name: hex-adr-create
description: Create a new Architecture Decision Record with auto-numbering
trigger: /hex-adr-create
---

# Create New ADR

## Steps

1. Find the highest ADR number in `docs/adrs/`:
   ```bash
   ls docs/adrs/ADR-*.md docs/adrs/adr-*.md 2>/dev/null | sort -t- -k2 -n | tail -1
   ```

2. Increment the number by 1, zero-pad to 3 digits

3. Ask the user for:
   - Title (required)
   - Brief context description

4. Copy `docs/adrs/TEMPLATE.md` to `docs/adrs/adr-{NNN}-{kebab-slug}.md`

5. Fill in:
   - Title: `# ADR-{NNN}: {Title}`
   - Status: `**Status:** Proposed`
   - Date: today's date
   - Drivers: from user input
   - Context section: from user input

6. Open the new file for editing

## Example

User: `/hex-adr-create`
Assistant: "What architectural decision needs to be recorded?"
User: "We should use WebSockets instead of polling for real-time updates"
-> Creates `docs/adrs/adr-044-websocket-realtime.md` with template filled in
