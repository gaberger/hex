// Re-export shared tool types from hex-core
pub use hex_core::domain::tools::{ToolCall, ToolDefinition, ToolInputSchema, ToolResult};

/// The built-in tools that hex-agent provides to the LLM.
/// These mirror Claude Code's tool set for compatibility.
pub fn builtin_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "read_file".into(),
            description: "Read a file from the filesystem. Returns the file contents with line numbers.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "path": { "type": "string", "description": "Absolute or relative path to the file" },
                    "offset": { "type": "integer", "description": "Line number to start reading from (optional)" },
                    "limit": { "type": "integer", "description": "Number of lines to read (optional)" }
                }),
                required: vec!["path".into()],
            },
        },
        ToolDefinition {
            name: "write_file".into(),
            description: "Write content to a file, creating it if it doesn't exist.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "path": { "type": "string", "description": "Path to the file to write" },
                    "content": { "type": "string", "description": "Content to write" }
                }),
                required: vec!["path".into(), "content".into()],
            },
        },
        ToolDefinition {
            name: "edit_file".into(),
            description: "Replace an exact string in a file with new content.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "path": { "type": "string", "description": "Path to the file" },
                    "old_string": { "type": "string", "description": "Exact string to find and replace" },
                    "new_string": { "type": "string", "description": "Replacement string" }
                }),
                required: vec!["path".into(), "old_string".into(), "new_string".into()],
            },
        },
        ToolDefinition {
            name: "glob_files".into(),
            description: "Find files matching a glob pattern.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "pattern": { "type": "string", "description": "Glob pattern (e.g., 'src/**/*.rs')" },
                    "path": { "type": "string", "description": "Base directory to search in (optional)" }
                }),
                required: vec!["pattern".into()],
            },
        },
        ToolDefinition {
            name: "grep_search".into(),
            description: "Search file contents using a regex pattern.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "pattern": { "type": "string", "description": "Regex pattern to search for" },
                    "path": { "type": "string", "description": "Directory or file to search in (optional)" },
                    "glob": { "type": "string", "description": "File glob filter (e.g., '*.rs')" }
                }),
                required: vec!["pattern".into()],
            },
        },
        ToolDefinition {
            name: "bash".into(),
            description: "Execute a bash command and return its output.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "command": { "type": "string", "description": "The bash command to execute" },
                    "timeout": { "type": "integer", "description": "Timeout in seconds (default 120)" }
                }),
                required: vec!["command".into()],
            },
        },
        ToolDefinition {
            name: "list_directory".into(),
            description: "List files and directories at a given path.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "path": { "type": "string", "description": "Directory path to list" }
                }),
                required: vec!["path".into()],
            },
        },
        ToolDefinition {
            name: "worktree_create".into(),
            description: "Create an isolated git worktree for safe refactoring. All file changes happen in the worktree, not the main branch. Returns the worktree path.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "branch": { "type": "string", "description": "Branch name for the worktree (e.g., 'refactor/extract-traits')" },
                    "base": { "type": "string", "description": "Base branch to create from (default: current HEAD)" }
                }),
                required: vec!["branch".into()],
            },
        },
        ToolDefinition {
            name: "worktree_status".into(),
            description: "List all active worktrees with their branch, path, and status (clean/dirty).".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: vec![],
            },
        },
        ToolDefinition {
            name: "worktree_merge".into(),
            description: "Merge a worktree branch back into the base branch and clean up the worktree. Fails if tests don't pass.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "branch": { "type": "string", "description": "Worktree branch to merge" },
                    "target": { "type": "string", "description": "Target branch to merge into (default: main)" },
                    "verify_command": { "type": "string", "description": "Command that must pass before merge (e.g., 'cargo test')" }
                }),
                required: vec!["branch".into()],
            },
        },
        ToolDefinition {
            name: "worktree_remove".into(),
            description: "Remove a worktree and optionally delete its branch. Use after merge or to abandon changes.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "branch": { "type": "string", "description": "Worktree branch to remove" },
                    "delete_branch": { "type": "boolean", "description": "Also delete the branch (default: false)" }
                }),
                required: vec!["branch".into()],
            },
        },
        // ── Hex architecture tools ───────────────────────────
        ToolDefinition {
            name: "hex_analyze".into(),
            description: "Run hexagonal architecture health check. Reports boundary violations, dead exports, circular dependencies. Run before committing code changes.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "path": { "type": "string", "description": "Project root path to analyze (default: .)" }
                }),
                required: vec![],
            },
        },
        ToolDefinition {
            name: "hex_plan".into(),
            description: "Decompose requirements into adapter-bounded tasks following hexagonal architecture. Returns a workplan with dependency tiers.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "requirements": { "type": "string", "description": "Requirements to decompose (comma or newline separated)" },
                    "language": { "type": "string", "description": "Target language: typescript, rust, or go" }
                }),
                required: vec!["requirements".into()],
            },
        },
        ToolDefinition {
            name: "hex_summarize".into(),
            description: "Get a token-efficient AST summary of source files. Use this instead of reading full files when you need to understand structure without implementation details.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "path": { "type": "string", "description": "File or directory path to summarize" }
                }),
                required: vec!["path".into()],
            },
        },
        ToolDefinition {
            name: "hex_adr_search".into(),
            description: "Search Architecture Decision Records by keyword. Returns matching ADRs with context.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({
                    "query": { "type": "string", "description": "Search keyword" }
                }),
                required: vec!["query".into()],
            },
        },
        ToolDefinition {
            name: "hex_adr_list".into(),
            description: "List all Architecture Decision Records with their status.".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: vec![],
            },
        },
    ]
}
