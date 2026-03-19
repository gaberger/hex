# ADR-036: hex-chat Session Architecture

- **Status**: Proposed
- **Date**: 2026-03-19
- **Informed by**: OpenCode (anomalyco/opencode), ADR-035
- **Authors**: Gary (architect), Claude (analysis)

## Context

hex-nexus has a working chat WebSocket (`/ws/chat`) with an LLM bridge and agent relay, but conversations are **ephemeral** — they live in an `Arc<Mutex<Vec<Value>>>` tied to a single WebSocket connection. When the connection drops, history is gone.

OpenCode demonstrates that **persistent, session-based conversation management** is essential for a developer command center:

| Problem in hex today | OpenCode's solution |
|---|---|
| History lost on WS disconnect | SQLite-backed session + message tables |
| No way to resume a conversation | Session list + resume by ID |
| No conversation branching | Fork operation (new session with parentID) |
| Context grows unbounded | Compact/summarize operation |
| Single client type (browser) | Thin client protocol — TUI and web share the same API |
| No structured tool results | MessagePart types (text, tool_call, tool_result, file) |

ADR-035 envisions hex-chat as the "developer command center." This ADR defines the session persistence layer that makes that possible.

## Decision

### 1. Session Domain Model

```
Session (aggregate root)
├── id: SessionId (branded UUID)
├── parent_id: Option<SessionId>     // fork lineage
├── project_id: String               // scoped to project
├── title: String                    // auto-generated or user-set
├── model: String                    // primary model used
├── status: SessionStatus            // active | archived | compacted
├── created_at: DateTime
├── updated_at: DateTime
└── messages: Vec<Message>           // ordered by sequence
    ├── id: MessageId (branded UUID)
    ├── session_id: SessionId
    ├── role: Role                   // user | assistant | system | tool
    ├── parts: Vec<MessagePart>      // structured content
    │   ├── TextPart { content }
    │   ├── ToolCallPart { tool_name, arguments, call_id }
    │   ├── ToolResultPart { call_id, content, is_error }
    │   └── FilePart { path, language, snippet }
    ├── model: Option<String>        // model that generated this
    ├── token_usage: Option<TokenUsage>
    ├── sequence: u32                // ordering within session
    └── created_at: DateTime
```

### 2. Session Port (trait)

```rust
#[async_trait]
pub trait ISessionPort: Send + Sync {
    // Lifecycle
    async fn session_create(&self, project_id: &str, model: &str, title: Option<&str>) -> Result<Session, SessionError>;
    async fn session_get(&self, id: &SessionId) -> Result<Option<Session>, SessionError>;
    async fn session_list(&self, project_id: &str, limit: u32, offset: u32) -> Result<Vec<SessionSummary>, SessionError>;
    async fn session_update_title(&self, id: &SessionId, title: &str) -> Result<(), SessionError>;
    async fn session_archive(&self, id: &SessionId) -> Result<(), SessionError>;
    async fn session_delete(&self, id: &SessionId) -> Result<(), SessionError>;

    // Messages
    async fn message_append(&self, session_id: &SessionId, msg: NewMessage) -> Result<Message, SessionError>;
    async fn message_list(&self, session_id: &SessionId, limit: u32, before: Option<u32>) -> Result<Vec<Message>, SessionError>;

    // Operations
    async fn session_fork(&self, id: &SessionId, at_sequence: Option<u32>) -> Result<Session, SessionError>;
    async fn session_revert(&self, id: &SessionId, to_sequence: u32) -> Result<(), SessionError>;
    async fn session_compact(&self, id: &SessionId, summary: &str) -> Result<(), SessionError>;

    // Search
    async fn session_search(&self, project_id: &str, query: &str, limit: u32) -> Result<Vec<SessionSummary>, SessionError>;
}
```

### 3. Storage Strategy

**SQLite in `~/.hex/hub.db`** — extends the existing database with two new tables:

```sql
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    parent_id TEXT REFERENCES sessions(id),
    project_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active',  -- active | archived | compacted
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS session_messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL,           -- user | assistant | system | tool
    parts_json TEXT NOT NULL,     -- JSON array of MessagePart
    model TEXT,
    input_tokens INTEGER,
    output_tokens INTEGER,
    sequence INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_messages_session ON session_messages(session_id, sequence);
CREATE INDEX idx_sessions_project ON sessions(project_id, updated_at DESC);
```

### 4. Integration Points

**Chat WebSocket** (`/ws/chat`): Modified to accept an optional `session_id` query parameter. If provided, loads history on connect. All messages auto-persisted.

**REST API** (new routes under `/api/sessions`):
- `POST /api/sessions` — create
- `GET /api/sessions?project_id=X` — list
- `GET /api/sessions/:id` — get with recent messages
- `POST /api/sessions/:id/fork` — fork
- `POST /api/sessions/:id/compact` — compact
- `DELETE /api/sessions/:id` — delete

**CLI** (`hex chat`):
- `hex chat` — start new session (TUI)
- `hex chat list` — list sessions
- `hex chat resume <id>` — resume session (TUI)
- `hex chat attach <url>` — connect to remote nexus
- `hex chat export <id>` — export as markdown

### 5. Compact Operation

Learned from OpenCode: long sessions degrade LLM performance. Compact replaces all messages before a threshold with a single system message containing a summary. The original messages are preserved in a `session_messages_archive` table for auditability.

### 6. What We Skip (vs. OpenCode)

| OpenCode feature | Decision | Reason |
|---|---|---|
| Vercel AI SDK | **Skip** | hex-agent already has RL-driven provider routing |
| Permission system (allow/ask/deny) | **Defer** | Existing auth token is sufficient for single-user |
| OpenTUI framework | **Skip** | Use ratatui — hex-cli is Rust, not TypeScript |
| Drizzle ORM | **Skip** | Use rusqlite directly — already used in hex-nexus |
| Structured output with retry | **Defer** | Useful but not needed for MVP |
| OAuth MCP flow | **Defer** | hex MCP tools work via stdio today |

## Consequences

### Positive
- Conversations survive restarts, reconnects, and crash recovery
- Fork enables "what if" branching (try a different approach, keep the original)
- Compact prevents context window degradation in long sessions
- Same protocol serves TUI and web clients (thin client principle from OpenCode)
- Session scoping by project_id keeps conversations organized

### Negative
- SQLite writes add latency (~1ms per message insert — negligible)
- Migration needed for existing hub.db (additive, non-breaking)
- `parts_json` as a JSON column sacrifices relational querying of individual parts

### Risks
- Session table growth for heavy users → mitigate with `session_archive` + TTL cleanup
- Fork chains can get deep → limit fork depth to 5
