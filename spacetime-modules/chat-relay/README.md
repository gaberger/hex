# chat-relay

> Agent-to-agent conversations + persistent chat sessions (ADR-036 / ADR-042 P2.5).

Two coexisting concerns in one module:

1. **Conversation/Message** — original agent-to-agent chat surface (broadcasts, simple message log).
2. **ChatSession** — durable user sessions with branchable history, sequence numbering, archive support.

## Tables

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `conversation` | public | `id` (unique) | Top-level chat thread for an agent (`agent_id`, `agent_name`, `archived`) |
| `message` | public | `id` (unique) | Message in a conversation — `conversation_id`, `role`, `sender_name`, `content`, `timestamp` |
| `chat_session` | public | `id` (unique) | Persistent user session — `parent_id` (forking), `project_id`, `title`, `model`, `status` (`active`/`archived`/`compacted`) |
| `chat_session_message` | public | `id` (unique) | One message in a session — `parts_json` (MessagePart[]), `role`, `model`, `input_tokens`, `output_tokens`, `sequence` |
| `chat_session_message_archive` | public | `id` (unique) | Archived messages with `archived_at` (compaction target) |

## Reducers

### Agent conversations

| Reducer | Args | Effect |
|---|---|---|
| `create_conversation` | `id, agent_id, agent_name` | Insert new conversation |
| `archive_conversation` | `conversation_id` | Set `archived = true` |
| `send_message` | `conversation_id, role, sender_name, content` | Append message; errors if conversation missing |
| `clear_conversation` | `conversation_id` | Delete all messages |

### Chat sessions

| Reducer | Args | Effect |
|---|---|---|
| `session_create` | `id, parent_id, project_id, title, model` | Insert session (status=`active`) |
| `session_update_title` | `session_id, title` | Rename session |
| `session_set_status` | `session_id, status` | Transition active/archived/compacted |
| `session_delete` | `session_id` | Cascade delete session + messages |
| `session_message_append` | `session_id, id, role, parts_json, model, input_tokens, output_tokens, sequence, created_at` | Append message at given sequence |
| `session_revert` | `session_id, to_sequence` | Truncate messages past sequence (branching point) |
| `session_archive_messages` | `session_id, before_sequence, archived_at` | Move old messages to archive table |
| `session_insert_forked` | (forked-message args) | Insert message into a forked session |

## Subscriptions

```sql
SELECT * FROM chat_session WHERE project_id = ? ORDER BY updated_at DESC
SELECT * FROM chat_session_message WHERE session_id = ? ORDER BY sequence ASC
SELECT * FROM message WHERE conversation_id = ? ORDER BY timestamp ASC
```

## Forking model

`chat_session.parent_id` (empty string = no parent) lets sessions branch. `session_insert_forked` copies messages from a parent at a given sequence — used by the dashboard "fork from here" button. `session_revert` is the destructive variant.

## Compaction

`session_archive_messages(before_sequence, archived_at)` moves old messages from `chat_session_message` → `chat_session_message_archive`. The active table stays small; archived rows remain queryable but excluded from default subscriptions. Set `chat_session.status = 'compacted'` afterward.
