Performs exact string replacements in files.

You MUST use the Read tool at least once in the conversation before editing. When editing text from Read tool output, preserve the exact indentation (tabs/spaces) as it appears AFTER the line number prefix.

The edit will FAIL if old_string is not unique in the file. Either provide more surrounding context to make it unique, or use replace_all to change every instance.

## hex-specific rules

### Always read before edit

This is not optional. Edit matches `old_string` byte-for-byte — if you haven't read the current file, you will mismatch indentation, miss recent changes, or corrupt the file.

### Uniqueness requirement

If `old_string` appears more than once, Edit will fail. Solutions:
1. Expand `old_string` to include more surrounding context (function signature, struct name, comment above)
2. Use `replace_all: true` if you intentionally want every occurrence changed (e.g., renaming a variable)

### Hexagonal architecture edit discipline

When editing an adapter, keep the edit within that adapter's boundary:
- Do NOT edit a port interface from inside an adapter file — open the port file separately
- Do NOT import other adapter modules while editing an adapter
- If your edit requires touching 2+ architectural layers, treat each as a separate Edit call

### Common edit targets in hex

```
# Port trait: add a new method signature
hex-agent/src/ports/prompt.rs

# Secondary adapter: implement a new method
hex-agent/src/adapters/secondary/prompt.rs
hex-agent/src/adapters/secondary/tools.rs

# Workplan executor: add phase handling
hex-nexus/src/orchestration/workplan_executor.rs

# Agent manager: update agent coordination
hex-nexus/src/orchestration/agent_manager.rs

# TUI: modify chat interface
hex-cli/src/tui/mod.rs
hex-cli/src/tui/session.rs
```

### After editing Rust files

Run `cargo check -p <crate>` to verify the edit compiles before moving on.
