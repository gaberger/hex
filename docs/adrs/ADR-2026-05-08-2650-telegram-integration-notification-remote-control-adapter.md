# ADR-2026-05-08-2650 — telegram-integration-notification-remote-control-adapter

Status: **Accepted** (shipped 2026-05; commits d327a266 telegram_notifier stub + dc08f6f5 wire into escalate_to_operator)
Date: 2026-05-09

---
Status: **Accepted** (shipped 2026-05; commits d327a266 telegram_notifier stub + dc08f6f5 wire into escalate_to_operator)  
Supersedes: -  
Superseded-By: -  
---

# ADR-2026-05-08-2650: Telegram Integration as Notification and Remote-Control Adapter

## Context

Hex currently delivers agent notifications exclusively through the browser-based inbox API (`hex-nexus/src/routes/inbox.rs:30`, `hex-cli/src/commands/inbox.rs:60`). Priority-2 critical notifications (SOP failures, watchdog autorestart events, cost budget overruns) require operator acknowledgment within 5 minutes (ADR-060 SLA), but the dashboard must be open for the operator to see them. When nexus is down or the operator is mobile, critical alerts are invisible until the next manual check.

The operator requests an out-of-band notification channel with mobile-friendly remote-control capabilities: acknowledge inbox items, check system status, queue intents via `hex hey`, and abort runaway tasks—all from a chat interface that works when the primary dashboard is unreachable.

Telegram provides:
- Mobile push notifications (always-on, cross-platform)
- Bot API with long-polling or webhook delivery
- Command interface (`/status`, `/ack <id>`, `/queue <intent>`)
- Strong operator authentication (chat_id allowlist)
- Self-hosted option (no cloud lock-in)

**Why now**: Recent SOP failure during a nexus restart went unnoticed for 18 minutes because the operator was traveling. An out-of-band paging surface would have surfaced the P2 notification immediately.

**Threat model**: Telegram-server compromise exposes chat history (operator commands + hex system responses). This is an acceptable tradeoff for operational resilience—hex already stores task payloads and cost data in SpacetimeDB; Telegram adds no new sensitive state. Bot token leakage is mitigated by storing it in the existing secret-grant module (`hex-nexus/src/adapters/spacetime_secrets.rs:100`) with AES-256-GCM vault encryption. Rate limits (5 commands/minute per chat_id) prevent abuse if a chat_id is compromised.

## Decision

We will implement a Telegram Bot API secondary adapter (`hex-nexus/src/adapters/telegram_bot.rs`) that:

1. **Bot API only** (not MTProto): Telegram Bot API provides webhook + long-polling with HTTPS transport and no E2E encryption complexity. MTProto offers E2E encryption, but bot messages are server-stored anyway (Telegram cloud) and MTProto client libraries (e.g., grammers) require phone-number registration. Bot API is the standard for server-side notification bots.

2. **Long polling for MVP**: Long polling (`getUpdates` with offset + timeout) requires no public endpoint, no TLS cert provisioning, and survives nexus IP changes. Webhook mode can be added later when hex runs behind a stable public domain. The adapter will call `getUpdates?timeout=30` in a loop from a Tokio background task spawned in `hex-nexus/src/main.rs` after SpacetimeDB initialization.

3. **Rust library**: **teloxide** (https://github.com/teloxide/teloxide) — mature, actively maintained, strong types for Bot API 7.x, built on reqwest + tokio. Alternatives: frankenstein (lower-level, more boilerplate); raw reqwest (reinvents command routing). Teloxide's `Dispatcher` matches hex's command-pattern architecture.

4. **Secret storage**: `HEX_TELEGRAM_BOT_TOKEN` stored in SpacetimeDB secret-grant module (same pattern as `hex-nexus/src/adapters/spacetime_secrets.rs:130`). Operator sets token via:
   ```bash
   hex secret grant --name telegram_bot_token --value <token> --scope nexus
   ```
   Adapter reads from secret-grant on startup; if missing, Telegram integration is disabled (graceful degradation).

5. **Adapter location**: `hex-nexus/src/adapters/telegram_bot.rs` (secondary adapter). Registered in `hex-nexus/src/adapters/mod.rs:22` as `pub mod telegram_bot;`. Struct `TelegramBotAdapter` holds:
   - `Arc<dyn IStatePort>` for inbox queries
   - `Arc<dyn ISecretGrantPort>` for token retrieval
   - `teloxide::Bot` instance
   - `HashSet<i64>` for chat_id allowlist (loaded from `.hex/telegram_allowed_chats.json` or env `HEX_TELEGRAM_ALLOWED_CHAT_IDS="123,456"`)

6. **Opt-in activation**: Operator creates `.hex/telegram.toml`:
   ```toml
   enabled = true
   allowed_chat_ids = [123456789]  # operator's Telegram chat_id
   rate_limit_per_minute = 5
   ```
   If file absent or `enabled = false`, adapter is not spawned.

7. **Kill switch**: `HEX_DISABLE_TELEGRAM=1` env var bypasses all Telegram logic (for incident response or security advisory).

## Capability Matrix

**Outbound notifications** (hex → Telegram):
- Priority-2 inbox notifications (SOP failure, watchdog autorestart, cost budget overage) — hooked into `hex-nexus/src/routes/inbox.rs:50` after `port.inbox_notify()`. Adapter subscribes to a notification channel (new `tokio::sync::broadcast` in `hex-nexus/src/state.rs`) and sends formatted message to all allowed chat_ids.
- Daily cost report (scheduled by `hex-nexus/src/orchestration/resource_observer.rs` if implemented).
- ADR conformance violations (optional: streamed from `hex-nexus/src/orchestration/adr_conformance.rs:80` when detector finds a critical drift).

**Inbound commands** (Telegram → hex):
- `/status` — returns nexus uptime, active agents, pending inbox count (calls `GET /api/inbox?min_priority=2`)
- `/inbox` — lists unacked P2 notifications (renders first 5 as Telegram message)
- `/ack <id>` — acknowledges notification (calls `POST /api/inbox/{id}/ack` via `hex-nexus/src/routes/inbox.rs:120`)
- `/queue <intent>` — enqueues intent via existing `hex hey` routing (`hex-cli/src/commands/hey.rs` → `POST /api/twin/directive`)
- `/abort <task_id>` — kills a runaway swarm task (calls hypothetical `POST /api/swarm/tasks/{id}/abort`)

Each command is rate-limited (5/min per chat_id, enforced by a `HashMap<i64, VecDeque<Instant>>` in the adapter). Commands from non-allowlisted chat_ids receive "Unauthorized" and are logged to `hex-nexus` structured logs (potential security event).

## Authentication Model

**Chat ID allowlist**: Operator determines their Telegram chat_id by messaging the bot (`/start`); bot replies with "Your chat_id: 123456789". Operator adds this to `.hex/telegram.toml` or env var. Only messages from allowlisted chat_ids are processed.

**HMAC (optional Phase 2)**: Commands can include a signature: `/ack 42 --sig <HMAC-SHA256(command || timestamp, shared_secret)>`. Not required for MVP because chat_id allowlist + Telegram's server-side auth is sufficient for a single-operator deployment.

**Rate limits**: 5 commands/minute per chat_id. Exceeding the limit returns "Rate limit exceeded, try again in Xs" and does not execute the command.

## Threat Model

| Threat | Mitigation | Residual Risk |
|--------|------------|---------------|
| Bot token leakage | Stored in SpacetimeDB secret-grant (AES-256-GCM encrypted), read-only access, rotatable via `hex secret grant --name telegram_bot_token --value <new>` | Attacker with STDB access can extract vault key from env; treat `HEX_VAULT_KEY` as crown jewel |
| Telegram server compromise | Chat history visible to Telegram (commands + responses); no PII or credentials in messages (task_id and notification_id are UUIDs) | Acceptable; operator already trusts Telegram for mobile infra |
| Chat_id spoofing | Impossible; Telegram server guarantees chat_id authenticity | None |
| Allowlist bypass | Only allowlisted chat_ids processed; unauthorized attempts logged | Operator must protect `.hex/telegram.toml` file permissions (chmod 600) |
| Command injection | All commands parsed by teloxide's `BotCommands` derive macro (type-safe); no shell execution | None |
| Replay attack | No timestamp validation in MVP; attacker recording `/ack 42` can replay it | Phase 2: HMAC with timestamp; low priority because chat history compromise implies Telegram server breach (operator has bigger problems) |

This reuses the LLM06 secret-leakage lens from prior ADR work (SpacetimeDB secret-grant module design). Telegram integration adds no new secret-storage surface; it consumes the existing vault.

## Consequences

**Positive**:
- Out-of-band paging when dashboard is down (mobile push notifications for P2 inbox events)
- Faster incident response (acknowledge SOP failures from phone, no laptop required)
- Remote task control (`/abort <task_id>`) during runaway inference loops
- Lower MTTR for critical alerts (5min SLA becomes achievable even when operator is mobile)

**Negative**:
- External dependency (Telegram API availability; mitigated by graceful degradation—if Telegram is down, inbox API still works)
- New authentication surface (chat_id allowlist must be manually maintained; risk of operator error if `.hex/telegram.toml` is misconfigured)
- Chat history outside hex (commands + responses stored on Telegram servers; acceptable per threat model)
- Maintenance burden (teloxide major-version upgrades when Telegram Bot API changes)

**Risks**:
- Operator forgets to rotate bot token after laptop theft → attacker can send commands if they also steal `.hex/telegram.toml`. Mitigation: `hex secret rotate telegram_bot_token` CLI command (Phase 2).
- Rate limit too low → operator locked out during incident. Mitigation: make rate limit configurable in `.hex/telegram.toml`.

## Configuration

**Environment variables**:
- `HEX_TELEGRAM_BOT_TOKEN` — Bot API token (from @BotFather); stored in secret-grant, not raw env var in production
- `HEX_TELEGRAM_ALLOWED_CHAT_IDS` — Comma-separated list (fallback if `.hex/telegram.toml` missing)
- `HEX_DISABLE_TELEGRAM=1` — Global kill switch

**Config file** (`.hex/telegram.toml`):
```toml
enabled = true
allowed_chat_ids = [123456789, 987654321]
rate_limit_per_minute = 5
notify_on_priority = 2  # Forward inbox notifications >= priority 2
```

**CLI commands**:
- `hex telegram enable` — Creates `.hex/telegram.toml` with operator's chat_id (determined by `/start` handshake)
- `hex telegram disable` — Sets `enabled = false`
- `hex telegram status` — Shows bot connection status, allowed chat_ids, message count

## Alternatives Considered

1. **Slack/Discord**: Require OAuth, workspace/server setup, more complex auth (operator wants single-tenant, zero-admin). Telegram bot = one token, no workspace.

2. **Matrix (self-hosted)**: Full control, E2E encryption. Rejected: operator must run Matrix homeserver (extra infra). Telegram self-hosted bot API is lighter (single Docker container if needed).

3. **Signal**: No bot API (only Signal CLI wrapper, brittle). Rejected.

4. **Email-only**: SMTP + IMAP parsing for commands. Rejected: latency (email delivery delays), spam risk, no push notifications on mobile unless operator configures aggressive polling.

5. **Webhook mode first**: Requires public IP, TLS cert, DNS. Rejected for MVP: operator runs nexus on home network behind NAT. Long polling works today.

6. **MTProto client**: E2E encryption, but requires phone number, more complex client (grammers library). Rejected: Bot API is Telegram's official server-bot interface; messages stored on Telegram cloud anyway (no E2E gain for bot use case).

**Why Telegram first**: Lowest operator friction (install Telegram app, message @BotFather, done). Slack/Discord require workspace/server admin. Matrix requires homeserver. Email has latency + spam UX issues. Telegram bot API is the sweet spot for self-hosted, mobile-first, command-driven ops tooling.

## Implementation Notes

File locations (all paths relative to `hex-nexus/`):
- `src/adapters/telegram_bot.rs:1` — `TelegramBotAdapter` struct, `spawn_bot_loop()` function
- `src/adapters/mod.rs:22` — `pub mod telegram_bot;`
- `src/state.rs:40` — Add `notification_tx: tokio::sync::broadcast::Sender<InboxNotification>` to `AppState`
- `src/routes/inbox.rs:50` — After `port.inbox_notify()`, call `state.notification_tx.send(notif.clone())`
- `src/main.rs:120` — After SpacetimeDB init, if `.hex/telegram.toml` exists, call `telegram_bot::spawn_bot_loop(state.clone())`

Dependencies (`hex-nexus/Cargo.toml`):
```toml
teloxide = { version = "0.12", features = ["macros", "auto-send"] }
```

Estimated effort: **1-2 weeks** (P0: adapter skeleton + `/status` + outbound P2 notifications; P1: `/ack`, `/inbox`, `/queue`, `/abort`; P2: HMAC signatures, webhook mode, `hex telegram` CLI).

## References

- `hex-cli/src/commands/inbox.rs:60` — Inbox list + ack (reuse this logic for `/inbox` and `/ack`)
- `hex-nexus/src/routes/inbox.rs:30` — REST inbox API (adapter calls these endpoints)
- `hex-nexus/src/adapters/spacetime_secrets.rs:100` — Secret-grant pattern (adapter uses same for bot token)
- `hex-nexus/src/routes/org_comms.rs:80` — DM routing via IAgentCommPort (conceptually similar: external message → internal action)
- Telegram Bot API 7.10 docs: https://core.telegram.org/bots/api
- teloxide book: https://github.com/teloxide/teloxide
