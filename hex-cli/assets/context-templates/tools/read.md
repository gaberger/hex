Reads a file from the local filesystem. Use absolute paths, not relative.

By default reads up to 2000 lines from the beginning of the file. Use offset and limit parameters for large files. Results are returned using cat -n format with line numbers starting at 1.

Can read images, PDFs, and Jupyter notebooks.

## hex-specific rules

**You MUST read a file before editing it.** Edit requires the file's current content in context — skipping this causes incorrect diffs and failed edits.

### When to use Read vs other tools

| Situation | Use |
|---|---|
| About to Edit a file | Read first (required) |
| Exploring/analyzing many files | `mcp__hex__hex_batch_execute` or `mcp__hex__hex_analyze` |
| Finding files by pattern | Glob |
| Searching content across files | Grep |

### Large files

For files over 2000 lines, use `offset` and `limit` to read only the relevant section. Example: reading a specific impl block without loading the whole file.

### hex architecture file paths

Common files you'll read during hex development:

```
hex-cli/assets/context-templates/   # Prompt templates (this system)
hex-cli/assets/agents/hex/hex/      # Agent YAML definitions
hex-cli/assets/skills/              # Skill definitions
hex-agent/src/ports/                # Port traits
hex-agent/src/adapters/secondary/   # Secondary adapters (prompt, tools)
hex-nexus/src/orchestration/        # Workplan executor, agent manager
docs/adrs/                          # Architecture Decision Records
docs/workplans/                     # Feature workplans
```
