# hex-hub Chat UI Frontend Audit

**File**: `hex-nexus/assets/chat.html`
**Date**: 2026-03-18
**Scope**: Single-file SPA (~681 lines) with embedded CSS and JavaScript

---

## 1. Message Handlers

### 1.1 `handleMessage(msg)` — Central Dispatcher (line 349)

Routes incoming WebSocket messages by `msg.type`. Supports seven event types:
`connected`, `stream_chunk`, `tool_call`, `tool_result`, `token_update`, `agent_status`, `chat_message`.

The WebSocket `onmessage` handler (line 319) unwraps a `WsEnvelope` format (`{topic, event, data}`) into a flat `{type: event, ...data}` shape before dispatching. If the message is already flat (no `.event`), it passes through as-is.

**Edge cases not handled:**
- Unknown message types are silently dropped (no default case, no logging).
- Malformed JSON triggers a `console.error` but no user-visible feedback.
- No `stream_end` or `stream_start` event type exists; stream lifecycle is inferred from `agent_status` transitions to `"idle"`.

### 1.2 `handleStreamChunk(msg)` (line 451)

**Expected format**: `{ type: "stream_chunk", text: string }`

**DOM manipulation**:
1. If no active streaming element exists, creates a new assistant message element via `createMsgEl("assistant")`.
2. Concatenates `msg.text` onto `body.dataset.raw` (accumulator stored as a data attribute).
3. Re-renders the entire accumulated text through `renderMarkdown()` and sets via `innerHTML` on every chunk.
4. Adds the `streaming-cursor` CSS class (blinking green caret).
5. Calls `scrollToBottom()`.

**Issues**:
- **O(n^2) rendering**: Every chunk re-parses and re-renders the entire accumulated message. For long responses (thousands of tokens), this causes increasing lag. A diff-based or append-only strategy would be significantly more efficient.
- **`msg.text` could be undefined**: No guard — would concatenate `"undefined"` string into the raw buffer.
- **Markdown mid-stream corruption**: Partial markdown (e.g., an unclosed ` ``` ` fence) will produce broken HTML until the closing fence arrives. The regex-based renderer has no concept of "pending" blocks.

### 1.3 `handleToolCall(msg)` (line 474)

**Expected format**: `{ type: "tool_call", tool_name: string, tool_input: object }`

**DOM manipulation**:
1. Creates (or reuses) an assistant message element.
2. Removes the streaming cursor.
3. Builds a collapsible "tool card" with:
   - A header showing a gear icon, tool name (amber monospace), and a chevron toggle.
   - An expandable detail section with the tool input (JSON, truncated to 200 chars) and a "running..." placeholder for the result.
4. Appends the card to the assistant message body.

**Edge cases not handled**:
- No `tool_call_id` matching: tool results are matched to tool cards by `tool_name` (last card with that name). If two calls to the same tool are in-flight, the result always goes to the last one — earlier calls remain stuck on "running...".
- `msg.tool_input` could be undefined, causing `JSON.stringify(undefined)` to return `undefined` (not a crash, but ugly display).
- No timeout indicator: if a tool never returns, the card shows "running..." forever.

### 1.4 `handleToolResult(msg)` (line 533)

**Expected format**: `{ type: "tool_result", tool_name: string, content: string|object, is_error?: boolean }`

**DOM manipulation**:
1. Finds the last `.tool-card` with matching `data-tool` attribute using `CSS.escape()`.
2. If `is_error` is truthy, adds `error` class to the header (turns it red).
3. Replaces the result section content with truncated output (800 chars max).
4. Re-adds the streaming cursor to signal more content may follow.

**Edge cases not handled**:
- If `state.currentAssistantEl` is null (e.g., page refreshed mid-stream), the function silently returns.
- `CSS.escape` may not be available in older browsers (pre-2018), though this is unlikely to matter in practice.
- No way for the user to see the full, untruncated result. The 800-char truncation is hard-coded with no "show more" affordance.

### 1.5 `handleTokenUpdate(msg)` (line 556)

**Expected format**: `{ type: "token_update", total_input?: number, total_output?: number, input_tokens?: number, output_tokens?: number }`

**DOM manipulation**:
1. Updates `state.totalInput` and `state.totalOutput` (but only if the field is present — uses `||` which means a value of `0` would NOT update, preserving the stale value).
2. Computes percentage of a hard-coded 200,000 token budget.
3. Updates the SVG gauge (stroke-dashoffset animation), percentage text, and color (green < 60%, yellow < 85%, red >= 85%).
4. Updates per-turn and cumulative token counters in the sidebar.

**Issues**:
- **Falsy-zero bug**: `msg.total_input || state.totalInput` means if the server sends `total_input: 0` (e.g., on reset), the UI keeps the old value. Should use `msg.total_input != null ? msg.total_input : state.totalInput`.
- **Hard-coded budget**: `tokenBudget: 200000` is not configurable and may not match the actual model context window.
- `input_tokens` and `output_tokens` (per-turn values) also use `|| 0`, which is fine for display but loses the ability to distinguish "not provided" from "zero".

### 1.6 `handleAgentStatus(msg)` (line 576)

**Expected format**: `{ type: "agent_status", status?: string, detail?: string, model?: string }`

**DOM manipulation**:
1. Sets the status pill text and class (`idle` or `active`).
2. Optionally updates the project directory display and model badge.
3. If status is `"idle"` and a stream was active, calls `endStream()` to finalize the current assistant message.

**Issues**:
- Any status that is not exactly `"idle"` gets the `active` class, including error states like `"crashed"` or `"error"` — they would show green "active" styling.
- `endStream()` increments `state.turnCount`, so a spurious idle status event would inflate the turn counter.

### 1.7 `handleConnected(msg)` (line 361)

**Expected format**: `{ type: "connected", data?: { llmBridge?: boolean } }`

**DOM manipulation**: If `data.llmBridge` is truthy, changes the model badge to "LLM bridge" with green styling.

**Issues**:
- Only checks for `llmBridge`. Does not read or display the actual model name, version, or capabilities from the connection handshake.
- The connection status dot is updated in `ws.onopen`/`ws.onclose`, not here, so this handler is purely cosmetic.

### 1.8 `addAssistantMessage(msg)` (line 595)

Called for `chat_message` events (non-streaming complete messages).

**DOM manipulation**:
1. Calls `endStream()` first (finalizes any in-progress stream).
2. Creates a new assistant message element and sets its content via `renderMarkdown()`.

**Issues**:
- `endStream()` increments `turnCount` even if there was no active stream, meaning a `chat_message` event always increments the turn counter twice (once in `endStream`, once... actually no, `addAssistantMessage` does not increment). However, if there IS an active stream, that stream's turn is counted and then the chat_message creates a second message — the stream content and the chat_message content are treated as separate messages. This could be a protocol mismatch.

---

## 2. Rendering Pipeline

### 2.1 Streaming Assembly

1. `handleStreamChunk` accumulates raw text in `body.dataset.raw`.
2. On every chunk, the full accumulated text is passed through `renderMarkdown()`.
3. `renderMarkdown()` escapes the entire input via `esc()` (creates a text node, reads `.textContent` — safe XSS prevention), then applies regex replacements for markdown syntax.
4. The result is set via `body.innerHTML`.

### 2.2 Markdown Rendering (`renderMarkdown`, line 371)

A custom regex-based renderer supporting:
- Fenced code blocks (` ```lang ... ``` `)
- Inline code
- Bold (`**text**`)
- Italic (`*text*`)
- Links (`[text](url)`)
- Blockquotes (`> text`)
- Unordered lists (`- item` or `* item`)
- Line breaks (newlines become `<br>`)
- Cleanup pass: removes `<br>` inside `<pre>` and `<ul>` blocks

**Not supported**:
- Ordered lists (`1. item`)
- Headings (`# H1`, `## H2`, etc.)
- Horizontal rules (`---`)
- Tables
- Images (`![alt](src)`)
- Nested lists
- Task lists (`- [ ] item`)
- Strikethrough (`~~text~~`)

### 2.3 Code Highlighting

**There is none.** Code blocks get a `class="lang-XXX"` attribute but no syntax highlighting library is loaded. All code renders as plain monospace text in `#d4d4d4` color on a dark background.

### 2.4 Tool Call Display

Tool calls render as collapsible cards within the assistant message bubble. The input is shown as truncated JSON; the result section shows "running..." until a `tool_result` event arrives. Cards use click-to-expand interaction via CSS class toggling.

### 2.5 Copy Buttons

Code blocks get a "copy" button (positioned absolutely in the top-right corner) that uses `navigator.clipboard.writeText()`. The button text briefly changes to "copied!" on success.

---

## 3. State Management

### 3.1 Global State Object (line 249)

```javascript
var state = {
  ws: null,              // WebSocket instance
  connected: false,      // Connection status (NOTE: updated in ws.onopen/onclose, not read by UI)
  reconnectDelay: 1000,  // Exponential backoff (doubles up to 30s)
  reconnectTimer: null,  // setTimeout handle
  turnCount: 0,          // User + assistant turns (displayed in sidebar)
  streaming: false,      // Whether we're mid-stream
  currentAssistantEl: null, // DOM element for the in-progress assistant message
  authToken: null,       // Hub auth token (from URL hash, query param, or prompt)
  totalInput: 0,         // Cumulative input tokens
  totalOutput: 0,        // Cumulative output tokens
  tokenBudget: 200000    // Hard-coded context window budget
};
```

### 3.2 What Is NOT Tracked

- **Conversation ID**: No concept of multiple conversations or conversation switching.
- **Message history array**: Messages exist only as DOM elements. There is no in-memory model — you cannot serialize, search, or replay the conversation.
- **Agent identity**: No tracking of which agent is responding (only a generic "assistant" role).
- **Pending tool calls**: No map of in-flight tool calls by ID. Matching is done by tool name against DOM elements.
- **Error state**: No dedicated error tracking or retry logic for failed sends.
- **User preferences**: No settings persistence (theme, font size, etc.).

---

## 4. Missing UI Features for Production

### 4.1 Conversation Management
- No conversation list/history sidebar.
- No ability to create, rename, delete, or switch conversations.
- No persistent storage (conversations lost on page reload).
- No conversation search.

### 4.2 Agent Selection
- No dropdown or selector for choosing which agent to talk to.
- The model badge is display-only and updated by the server.
- No ability to change model, temperature, or system prompt from the UI.

### 4.3 File Handling
- No file attachment button or drag-and-drop zone.
- No image paste support.
- No file preview or download for tool results that produce files.

### 4.4 Token Budget Visualization
- The gauge exists but the budget is hard-coded at 200k.
- No visual warning when approaching the limit.
- No "cost so far" display.
- No per-message token breakdown.

### 4.5 Tool Call Approval Flow
- Tool calls execute automatically with no user approval gate.
- No "approve/deny" UI for dangerous operations (file writes, shell commands).
- No tool call filtering or whitelisting.

### 4.6 Error Display
- No toast/notification system for errors.
- WebSocket disconnection shows only a red dot — no explanatory message or manual reconnect button.
- Failed sends (when disconnected) are silently dropped.

### 4.7 Other Missing Features
- No message editing or regeneration.
- No stop/cancel button for in-progress responses.
- No system message display.
- No keyboard shortcuts beyond Enter/Shift+Enter.
- No dark/light theme toggle (dark only).
- No export/download conversation feature.
- No typing indicator.
- No read receipts or delivery confirmation.
- No multi-turn context window management (no ability to "forget" or "pin" messages).

---

## 5. Bugs and Issues

### 5.1 Confirmed Bugs

1. **Falsy-zero token bug** (line 557-558): `msg.total_input || state.totalInput` — if the server sends `total_input: 0`, the UI keeps the stale value. This is a real bug that would prevent token counter resets.

2. **O(n^2) stream rendering** (line 457-458): Every chunk re-renders the entire message from scratch. For a 4,000-token response arriving in 200 chunks, this means ~200 full markdown parse-and-render passes of increasing size.

3. **Turn count double-increment risk**: `addUserMessage` increments `turnCount`. If `addAssistantMessage` is called (for `chat_message` type), it calls `endStream()` first which also increments `turnCount` — even if no stream was active. A single non-streaming exchange would count as 3 turns (user +1, endStream +1 spurious, but actually `endStream` only increments if `currentAssistantEl` exists... checking: no, `endStream` always increments unconditionally on line 469). So the count is inflated by 1 for every `chat_message` event.

4. **Tool result matching by name, not ID** (line 535): When multiple tool calls with the same name are in-flight, only the last card gets the result. Earlier cards remain stuck on "running..." forever.

5. **`esc()` function is a no-op** (line 277-281): The function creates a text node, appends it to a div, then reads `.textContent` — but `.textContent` on the parent returns the same string that was put in. This function returns its input unchanged. It does NOT escape HTML entities. The intent was likely to use `.innerHTML` to read back the escaped version. This means **the XSS protection is broken** — raw HTML in messages would be interpreted as markup after the regex transforms in `renderMarkdown`.

   Wait — re-reading: `document.createTextNode(s)` creates a text node. Appending it to a div. Then `d.textContent` returns the text content of the div, which is the text of the text node, which is the original string `s`. So yes, `esc(s)` returns `s` unchanged. The escaping does not happen. However, because `renderMarkdown` then does regex replacements that only add specific HTML tags, and the input text doesn't go through any HTML parsing step that would interpret embedded tags... actually, the result IS set via `innerHTML` on line 423. So if `msg.text` contains `<script>alert(1)</script>`, `esc()` returns it unchanged, the regexes don't touch it, and it gets set as `innerHTML`. **This is an XSS vulnerability.**

   **Correction**: Actually, `d.textContent` when reading back DOES return the raw text — it doesn't HTML-encode. But `document.createTextNode(s)` followed by reading the parent's `textContent` just gives back `s`. The standard pattern for escaping is: set `d.textContent = s` then read `d.innerHTML`. The current code doesn't do that. **Confirmed: `esc()` is a no-op and the XSS sanitization is ineffective.**

   **SEVERITY: HIGH** — Any user or LLM-generated content containing HTML tags will be rendered as HTML, enabling script injection.

6. **Reconnect delay never resets on failure** (line 338): `reconnectDelay` doubles on each `ws.onclose` but is only reset to 1000 on successful `ws.onopen` (line 308). If the server is down for a while, after reconnection the delay is properly reset. This is actually correct behavior.

7. **RL stats polling continues when disconnected** (line 672): `setInterval(fetchRLStats, 15000)` runs unconditionally. When disconnected, every 15 seconds a `fetch("/api/rl/stats")` fires and silently fails. Minor resource waste.

### 5.2 Race Conditions

1. **Stream chunk after endStream**: If a `stream_chunk` arrives after `agent_status: idle` (network reordering), it starts a new assistant message element, creating an orphaned partial message.

2. **Rapid reconnection**: If `connect()` is called while a WebSocket is still in CONNECTING state, a second socket is created. The old one's `onclose` will fire later and schedule yet another reconnect. No guard against multiple simultaneous connection attempts.

3. **Send during reconnect**: `wsSend()` checks `ws.readyState !== 1` and silently drops the message. The user sees their message in the chat but it was never sent. No retry queue, no error indication.

### 5.3 Memory Leaks

- DOM elements are only added, never removed (except on `/clear`). Long conversations will accumulate thousands of DOM nodes with no virtualization.
- `body.dataset.raw` stores the full text of every streamed message as a data attribute, doubling memory usage for streamed content.

---

## 6. Accessibility Audit

### 6.1 Keyboard Navigation

- **Enter to send**: Works. Shift+Enter for newline: Works.
- **Tab order**: The textarea and send button are focusable. Sidebar buttons are focusable. Tool card headers are clickable `<div>`s, NOT buttons — they are **not keyboard-accessible** (no `tabindex`, no `role="button"`, no keydown handler for Enter/Space).
- **No focus management**: When a new message appears, focus stays on the textarea. This is reasonable for a chat UI.
- **No skip navigation**: No skip-to-content link.
- **Sidebar toggle**: Is a `<button>` with `aria-label="Toggle sidebar"` — correct.

### 6.2 Screen Reader Support

- **No ARIA live regions**: New messages are appended to the DOM but there is no `aria-live="polite"` on the messages container. Screen readers will not announce new messages.
- **No ARIA roles**: The messages area has no `role="log"` or `role="feed"`. The sidebar has no `role="complementary"`.
- **Tool cards**: No `aria-expanded` attribute on the expandable cards. Screen readers cannot determine expand/collapse state.
- **Token gauge SVG**: The gauge uses `<text>` elements inside SVG but has no `aria-label` or `role="img"` on the `<svg>`. The percentage is readable as text content but the semantics are lost.
- **Status pill**: No `aria-live` on the agent status — changes won't be announced.
- **Connection status**: The dot uses visual color only. The text label ("connected"/"disconnected") is present, which helps, but has no `aria-live`.

### 6.3 Color Contrast

Tested against WCAG 2.1 AA (4.5:1 minimum for normal text):

| Element | Foreground | Background | Approx Ratio | Pass? |
|---------|-----------|------------|-------------|-------|
| Body text (`--text: #e0e0e0`) | #e0e0e0 | #0a0a0f | ~15:1 | Yes |
| Secondary text (`--text2: #9e9eb0`) | #9e9eb0 | #0a0a0f | ~7.5:1 | Yes |
| Tertiary text (`--text3: #6a6a80`) | #6a6a80 | #0a0a0f | ~3.8:1 | **No** |
| Input hint text (`--text3`) | #6a6a80 | #141420 | ~3.2:1 | **No** |
| Accent on dark (`--accent: #00d4aa`) | #00d4aa | #0a0a0f | ~10:1 | Yes |
| Tool name amber (`--amber: #ffb347`) | #ffb347 | amber-dim bg | ~3.5:1 | **No** |
| Inline code (`#d4d4ff`) | #d4d4ff | rgba(255,255,255,.07) on #1a1a28 | ~9:1 | Yes |

Three elements fail WCAG AA contrast requirements. The `--text3` color is used for: message role labels, input placeholder, input hint, sidebar section titles, token detail labels, and RL confidence text.

### 6.4 Motion and Preferences

- No `prefers-reduced-motion` media query. The blinking cursor and pulse animations will play regardless of user preference.
- No `prefers-color-scheme` support (dark mode only).
- No font size adjustment controls.

---

## 7. Security Notes

1. **XSS via broken `esc()` function**: As detailed in section 5.1 item 5, the `esc()` function is a no-op. Content set via `renderMarkdown()` -> `innerHTML` is not sanitized. An attacker who can inject content into the chat stream (compromised LLM response, malicious tool result) can execute arbitrary JavaScript.

2. **Auth token in URL**: The token is stored in `location.hash` and sent as a query parameter in the WebSocket URL. The hash is not sent to the server in HTTP requests (good), but the query parameter in the WS URL may be logged by proxies. The token is also visible in browser history and can be leaked via the Referer header if the page has outbound links.

3. **No CSP**: The page has no Content-Security-Policy meta tag. Combined with the XSS vector, this means injected scripts have full access.

4. **Link injection**: `renderMarkdown` converts `[text](url)` to `<a href="url">` with `target="_blank" rel="noopener"` — the `rel="noopener"` is good, but there is no URL validation. `javascript:` URLs would be rendered as clickable links.

---

## 8. Summary of Findings

### Critical
- **XSS vulnerability**: `esc()` function does not escape HTML. All content rendered via `innerHTML` is unsanitized.
- **No CSP headers** in the HTML document.

### High
- **O(n^2) stream rendering** will cause visible UI lag on long responses.
- **Tool call matching by name** breaks with concurrent same-name tool calls.

### Medium
- Falsy-zero token counter bug.
- Turn count inflation on `chat_message` events.
- Silent message dropping when disconnected.
- No ARIA live regions for screen readers.
- Three color contrast failures (WCAG AA).

### Low
- No syntax highlighting for code blocks.
- No conversation persistence or history.
- No reduced-motion support.
- RL polling continues when disconnected.
- Memory accumulation in long sessions.

### Architecture
The UI is a minimal but functional chat interface suitable for development/demo purposes. For production use, it needs: a proper markdown library (or fixing the custom one), conversation persistence, tool call approval flows, proper accessibility, and critically, XSS remediation.
