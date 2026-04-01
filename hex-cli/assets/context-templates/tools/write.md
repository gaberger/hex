Writes a file to the local filesystem. This tool WILL OVERWRITE existing files at the provided path.

If this is an existing file, you MUST use the Read tool first to read the file's contents. Prefer Edit for modifying existing files — it only sends the diff. Only use Write for new files or complete rewrites.

## hex-specific rules

### When to use Write vs Edit

| Situation | Use |
|---|---|
| Creating a new file | Write |
| Completely rewriting a file | Write (Read first) |
| Changing a specific section | Edit (more precise, less context used) |
| Changing a single function/struct | Edit |

### File placement rules

**NEVER save files to the root folder.** All files belong in their architectural layer:

| Content | Directory |
|---|---|
| Domain types, value objects | `hex-core/src/types/` or `domain/` |
| Port traits (interfaces) | `hex-agent/src/ports/` |
| Secondary adapters | `hex-agent/src/adapters/secondary/` |
| Primary adapters | `hex-agent/src/adapters/primary/` |
| Agent YAML definitions | `hex-cli/assets/agents/hex/hex/` |
| Skill definitions | `hex-cli/assets/skills/` |
| Context templates | `hex-cli/assets/context-templates/` |
| ADRs | `docs/adrs/` |
| Workplans | `docs/workplans/` |
| Specs | `docs/specs/` |
| Tests | `tests/unit/` or `tests/integration/` |

### Never commit secrets

Do not write `.env` files containing real credentials. Use `.env.example` with placeholders.

### Rust file conventions

- New Rust source files must be declared in `mod.rs` or `lib.rs` of their parent module
- Use `pub use` re-exports in `mod.rs` to maintain clean module boundaries
