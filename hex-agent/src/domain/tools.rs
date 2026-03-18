use serde::{Deserialize, Serialize};

/// Definition of a tool available to the agent — sent to Anthropic API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: ToolInputSchema,
}

/// JSON Schema for tool input parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInputSchema {
    #[serde(rename = "type")]
    pub schema_type: String,
    #[serde(default)]
    pub properties: serde_json::Value,
    #[serde(default)]
    pub required: Vec<String>,
}

/// A tool call from the assistant — extracted from tool_use content blocks.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// Result of executing a tool — fed back as tool_result content block.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

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
    ]
}
