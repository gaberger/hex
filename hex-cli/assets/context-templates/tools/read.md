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
.claude/agents/                     # Agent YAML definitions
.claude/skills/                     # Skill definitions
src/core/ports/                     # Port interfaces / traits
src/core/domain/                    # Domain types
src/adapters/                       # Adapter implementations
docs/adrs/                          # Architecture Decision Records
docs/workplans/                     # Feature workplans
```
