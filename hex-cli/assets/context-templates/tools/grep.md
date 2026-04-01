Powerful search tool built on ripgrep. ALWAYS use Grep for search tasks — NEVER invoke grep or rg as a Bash command.

Supports full regex syntax (e.g., "log.*Error"). Filter files with glob or type parameters. Use output_mode: "content" to see matching lines, "files_with_matches" for file paths only.

For open-ended searches requiring multiple rounds, use the Agent tool.

## hex-specific patterns

### Finding architectural elements

```
# Find a port trait definition
pattern: "pub trait PromptPort"
type: "rust"

# Find all impl blocks for a port
pattern: "impl.*PromptPort"
type: "rust"

# Find cross-adapter imports (architecture violation check)
pattern: "use.*adapters::(primary|secondary)"
type: "rust"

# Find all pub fn in a port file
pattern: "pub (async )?fn "
glob: "hex-agent/src/ports/*.rs"
output_mode: "content"
```

### Finding workplan/template references

```
# Find template variable usages
pattern: "\\{\\{[a-z_]+\\}\\}"
glob: "hex-cli/assets/context-templates/**/*.md"
output_mode: "content"

# Find a specific ADR reference in code
pattern: "ADR-[0-9]+"
type: "rust"
output_mode: "content"
```

### SpacetimeDB and HexFlo

```
# Find all reducer calls
pattern: "ctx\\.db\\."
glob: "spacetime-modules/**/*.rs"

# Find task state references
pattern: "HexFloTask|hexflo_task"
type: "rust"
output_mode: "files_with_matches"
```

### Output mode guide

| Need | output_mode |
|---|---|
| Which files contain a pattern | `files_with_matches` (default) |
| See the matching lines with context | `content` with `-C 2` |
| Count occurrences per file | `count` |

### head_limit

Default is 250 results. For broad searches across a large codebase, use `head_limit: 50` to stay focused. Pass `head_limit: 0` only when you genuinely need all matches.
