# ADR-2603221522: Embedded Asset Bundle — rust-embed for CLI Templates and Schemas

**Status:** Accepted
**Date:** 2026-03-22
**Drivers:** hex-cli uses scattered `include_str!` and hardcoded strings for templates (settings.json, project.json, CLAUDE.md scaffolds, workplan schema, ADR templates). Adding a new template requires touching Rust code and recompiling. hex-nexus already uses `rust-embed` successfully for dashboard assets.

## Context

hex-cli's `hex init` command scaffolds files into target projects: `.hex/project.json`, `.claude/settings.json`, `CLAUDE.md`, `docs/adrs/` templates, agent definitions, skill definitions, and hook scripts. Currently these are:

| Template | How it's embedded | Location |
|----------|------------------|----------|
| project.json | Hardcoded `format!()` in init.rs | `hex-cli/src/commands/init.rs` |
| settings.json | Hardcoded JSON string in init.rs | `hex-cli/src/commands/init.rs` |
| CLAUDE.md | Hardcoded string in init.rs | `hex-cli/src/commands/init.rs` |
| Workplan schema | `include_str!("../../../docs/workplan-schema.json")` | `hex-cli/src/commands/plan.rs` |
| ADR template | Hardcoded string in adr.rs | `hex-cli/src/commands/adr.rs` |
| StatusLine script | Hardcoded in hook.rs | `hex-cli/src/commands/hook.rs` |

**Problems:**
1. Adding a new template requires modifying Rust source and recompiling
2. Templates are scattered across multiple .rs files — no single inventory
3. No way to update templates without rebuilding the binary
4. Hardcoded strings are hard to review and test independently
5. hex-nexus solved this exact problem with `rust-embed` for dashboard assets

## Decision

Use `rust-embed` in hex-cli to embed a `hex-cli/assets/` directory tree into the binary. All templates, schemas, default configs, and scaffolding files live as real files in this directory — editable, reviewable, testable — and are baked into the binary at compile time.

### Directory Structure

```
hex-cli/assets/
  scaffold/                    # Files extracted by `hex init`
    .hex/
      project.json.tmpl       # Handlebars template with {{name}}, {{id}}
      adr-rules.toml           # Default ADR enforcement rules
    .claude/
      settings.json.tmpl      # Default Claude Code settings
      agents/                  # Default agent definitions
        planner.yml
        hex-coder.yml
        integrator.yml
      skills/                  # Default skill definitions (shipped with hex)
    docs/
      adrs/
        ADR-000-template.md    # ADR template
  schemas/
    workplan.schema.json       # Workplan JSON Schema (used by hex plan schema)
    project.schema.json        # Project manifest schema
    adr.schema.json            # ADR frontmatter schema
  templates/
    adr.md.tmpl                # ADR creation template
    claude-md.tmpl             # CLAUDE.md generation template
    statusline.cjs             # StatusLine hook script
```

### Embedding

```rust
// hex-cli/src/assets.rs
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "assets/"]
pub struct Assets;

impl Assets {
    /// Get a file's contents as a string.
    pub fn get_str(path: &str) -> Option<String> {
        Self::get(path).map(|f| String::from_utf8_lossy(&f.data).to_string())
    }

    /// Get a template and apply simple {{key}} substitutions.
    pub fn render_template(path: &str, vars: &[(&str, &str)]) -> Option<String> {
        let mut content = Self::get_str(path)?;
        for (key, value) in vars {
            content = content.replace(&format!("{{{{{}}}}}", key), value);
        }
        Some(content)
    }

    /// Extract all files under a prefix to a target directory.
    pub fn extract_to(prefix: &str, target: &std::path::Path) -> std::io::Result<Vec<String>> {
        let mut extracted = Vec::new();
        for path in Self::iter() {
            if path.starts_with(prefix) {
                let relative = &path[prefix.len()..];
                let dest = target.join(relative);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if let Some(file) = Self::get(&path) {
                    // Don't overwrite existing files (user may have customized)
                    if !dest.exists() {
                        std::fs::write(&dest, &file.data)?;
                        extracted.push(relative.to_string());
                    }
                }
            }
        }
        Ok(extracted)
    }
}
```

### Usage in Commands

```rust
// hex init — scaffold project
let created = Assets::extract_to("scaffold/", &project_dir)?;

// hex plan schema — output schema
let schema = Assets::get_str("schemas/workplan.schema.json")
    .ok_or_else(|| anyhow!("Schema not found in embedded assets"))?;
println!("{}", schema);

// hex adr create — new ADR from template
let template = Assets::render_template("templates/adr.md.tmpl", &[
    ("number", &format!("{:03}", next_number)),
    ("title", title),
    ("date", &today),
])?;
```

### Migration Path

1. **P1**: Add `rust-embed` to hex-cli/Cargo.toml, create `hex-cli/assets/` directory
2. **P1**: Move workplan-schema.json → `assets/schemas/workplan.schema.json`
3. **P2**: Extract hardcoded templates from init.rs → `assets/scaffold/` as `.tmpl` files
4. **P2**: Extract ADR template from adr.rs → `assets/templates/adr.md.tmpl`
5. **P3**: Extract statusline script, hook templates → `assets/templates/`
6. **P3**: Extract default agent/skill definitions → `assets/scaffold/.claude/`
7. **P4**: Update all `include_str!` and hardcoded strings to use `Assets::get_str()`
8. **P4**: Add `hex assets list` command to show all embedded assets (debugging aid)

## Consequences

### Positive
- **Single inventory** — all templates in one directory, easy to review and test
- **No Rust changes for template updates** — edit the file, rebuild, done
- **Consistent extraction** — `Assets::extract_to()` handles mkdir, skip-existing, reporting
- **Testable templates** — can validate JSON/TOML/YAML templates in CI without compiling Rust
- **Pattern consistency** — hex-nexus already uses rust-embed for dashboard, now hex-cli mirrors it

### Negative
- **Binary size increase** — templates are small (few KB total), negligible impact
- **Compile-time embedding** — templates can't be updated without rebuild (same as current)
- **Template syntax** — simple `{{key}}` substitution, not full Handlebars (keep it simple)

### Risks
- **Template path changes** — if assets/ directory is restructured, all callers need updating. Mitigated by the `Assets` wrapper API.

## References
- `rust-embed` crate: used by hex-nexus for dashboard assets
- ADR-049: Embedded Settings Template (related — settings.json as embedded default)
- hex-nexus `rust-embed` usage: `hex-nexus/src/embed.rs`
