Powerful search tool built on ripgrep. ALWAYS use Grep for search tasks — NEVER invoke grep or rg as a Bash command.

Supports full regex syntax (e.g., "log.*Error"). Filter files with glob or type parameters. Use output_mode: "content" to see matching lines, "files_with_matches" for file paths only.

For open-ended searches requiring multiple rounds, use the Agent tool.

## hex-specific patterns

### Finding architectural elements

```
# Find a port trait/interface definition
pattern: "pub trait .*Port"        # Rust
pattern: "interface .*Port"        # TypeScript
glob: "src/ports/*"
output_mode: "content"

# Find all implementations of a port
pattern: "impl.*MyPort"            # Rust
pattern: "implements MyPort"       # TypeScript
output_mode: "files_with_matches"

# Find cross-adapter imports (architecture violation check)
pattern: "use.*adapters::(primary|secondary)"   # Rust
pattern: "from.*adapters/(primary|secondary)"   # TypeScript
output_mode: "content"

# Find all public functions in port files
pattern: "pub (async )?fn |export (async )?function "
glob: "src/ports/*"
output_mode: "content"
```

### Finding workplan/template references

```
# Find template variable usages
pattern: "\\{\\{[a-z_]+\\}\\}"
glob: "**/*.md"
output_mode: "content"

# Find a specific ADR reference in code
pattern: "ADR-[0-9]+"
output_mode: "content"
```

### Output mode guide

| Need | output_mode |
|---|---|
| Which files contain a pattern | `files_with_matches` (default) |
| See the matching lines with context | `content` with `-C 2` |
| Count occurrences per file | `count` |

### head_limit

Default is 250 results. For broad searches across a large codebase, use `head_limit: 50` to stay focused. Pass `head_limit: 0` only when you genuinely need all matches.
