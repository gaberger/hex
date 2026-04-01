Fast file pattern matching tool that works with any codebase size. Supports glob patterns like "**/*.js" or "src/**/*.ts". Returns matching file paths sorted by modification time.

Use when you need to find files by name patterns. For open-ended searches requiring multiple rounds, use the Agent tool instead.

## hex-specific patterns

### Rust codebase

```
# Find all Rust source files in a crate
hex-agent/src/**/*.rs

# Find port traits
hex-agent/src/ports/*.rs

# Find secondary adapters
hex-agent/src/adapters/secondary/*.rs

# Find all mod.rs files (module declarations)
**/**/mod.rs

# Find all Cargo.toml files
**/Cargo.toml
```

### Assets and templates

```
# All context templates
hex-cli/assets/context-templates/**/*.md

# Agent YAML definitions
hex-cli/assets/agents/**/*.yml
hex-cli/assets/agents/**/*.yaml

# Skill definitions
hex-cli/assets/skills/*.md

# Swarm behavior YAMLs
hex-cli/assets/swarms/*.yml
```

### Docs

```
# All ADRs
docs/adrs/ADR-*.md

# All workplans
docs/workplans/feat-*.json

# All specs
docs/specs/*.json
```

### When Glob is NOT enough

If your search requires:
- Matching file *content* (not just names) → use Grep
- Multi-step exploration with unknown path structure → use Agent (Explore subtype)
- Finding where a specific function or struct is defined → use Grep with `type: "rust"`
