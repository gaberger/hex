# ADR-2607041205: research-dashboard — semantic chat over the knowledge base

**Status:** Completed
**Date:** 2026-07-04
**Epoch:** single-agent
**Drivers:** The operator wanted to ask questions of the research corpus (ADRs, specs, benchmarks,
analysis, etc.) in natural language rather than browsing/searching by filename, with the answer
grounded in and citing the actual docs — and to not lose that conversation on every server restart.
**Relates-To:** ADR-2607041035 (the research-dashboard app this extends)
**Supersedes:**
**Superseded-By:**

## Context

The existing filename-substring `search` (from ADR-2607041035's portal expansion) finds docs by name
match only — it can't answer "does X help Y" style questions or surface docs whose content is relevant
but whose filename isn't. The operator explicitly chose **semantic search (embeddings)** over extending
keyword search, accepting the added complexity (a local embedding model, an in-memory vector index)
for materially better retrieval on paraphrased questions — see the two-question `AskUserQuestion` this
session: retrieval strategy (embeddings, chosen over keyword) and answering model (`devstral-small-2:24b`,
chosen over `qwen3:4b` for quality over speed, since this is occasional Q&A not a hot loop).

## Decision

All new capability lives inside the existing hexagonal boundaries from ADR-2607041035 — no change to
the layer rules, only new ports/adapters within them.

- **Domain**: `ChatSource`, `ChatAnswer` (question, answer, sources, `answeredAt`).
- **Ports**: `IEmbedder` (driven — text → vector), `IChatModel` (driven — question + context chunks →
  answer text), `IKnowledgeIndex` (driven — semantic search, returns `KnowledgeMatch[]`),
  `IChatHistoryStore` (driven — append/list persisted Q&A). `IDashboardService` gained `askQuestion()`
  and `getChatHistory()`.
- **Secondary adapters**:
  - `ollama-embedder.ts` — calls local Ollama's `/api/embed` with `nomic-embed-text` (274MB, pulled
    this session; none was already available locally).
  - `ollama-chat-model.ts` — calls local Ollama's `/api/chat` with `devstral-small-2:24b`, a system
    prompt constraining it to answer only from the provided context and say so when it can't.
  - `embedding-knowledge-index.ts` — **no persistence, no vector DB**: chunks every doc's markdown by
    paragraph boundaries (~1500 chars/chunk), embeds each chunk via `IEmbedder`, caches in a plain
    in-memory array (2,395 chunks across all 9 collections), computes cosine similarity by hand (no
    library). Rebuilds from scratch on every process restart — acceptable since this is a dev-box
    side app restarted rarely, and rebuild is a background task that doesn't block the rest of the app
    from serving.
  - `sqlite-chat-history-store.ts` — **SQLite via `bun:sqlite`** (Bun's built-in module, zero new
    dependency), one `chat_history` table. Operator explicitly asked for SQLite over the initial
    JSON-file-append design once that was in place; swapped before ever committing the JSON version.
- **Primary adapter**: chat is now the **home page** (`/`) — operator's explicit call, moved from an
  initial separate `/chat` route (which now 302-redirects to `/`). `/system` absorbed the old home
  page's live-stats-plus-recent-docs content so nothing was lost in the move. `POST /api/chat` returns
  an htmx fragment appended to the chat log; history loads on page load so a refresh (or a restart)
  shows prior conversation, not a blank page.

New dependency: none beyond what ADR-2607041035 already introduced (Express, marked) — embeddings and
chat both go over HTTP to Ollama (already running locally), and SQLite is a Bun built-in.
`@types/bun` added as a dev dependency once `tsc --noEmit` couldn't resolve `bun:sqlite`'s types.

## Consequences

**Positive:**
- Correctness-safe failure mode: when the retrieved context doesn't contain the answer, the model says
  so explicitly rather than guessing — verified live, not assumed (see Negative below, same test run).
- Retrieval reliably finds the right document: asking about T1 speculative-decoding results surfaced
  the actual DSpark test plan at the top of results (cosine score 0.81) every time tested, and
  `analysis`/`benchmarks` collections both appear in real result sets alongside `adrs` — confirmed
  end-to-end, not just by code inspection.
- Conversation survives restarts: verified by asking a question, restarting the server (to pick up an
  unrelated code change), and reloading `/` — the prior Q&A was still there.
- Zero new external dependencies for either the RAG pipeline or persistence — Ollama was already
  running, SQLite ships inside Bun.

**Negative:**
- **Retrieval granularity is a real, observed limitation, not a hypothetical one.** Asking "does
  speculative decoding help hex's T1 tier" correctly retrieved the DSpark test plan (the right
  document) but the model still answered "not in the provided context" — the chunk(s) that scored
  highest were the test plan's objective/framing section, not its buried `Results` section where the
  actual FAIL verdict is stated in different vocabulary than the question. Paragraph-sized chunking
  with whole-corpus top-K selection doesn't guarantee the single most-informative paragraph of a long,
  dense document gets retrieved over that same document's intro. This was tried once (allowing 2 chunks
  per doc instead of 1, upping topK 5→8) and the fix wasn't sufficient for this specific query — not
  further pursued this session past that one iteration.
- In-memory index means every restart pays a ~2-3 minute rebuild cost (2,395 embedding calls,
  sequential) before `/` gives fully warmed answers; earlier questions during that window still work,
  just wait on the same shared build promise.
- No auth (same as ADR-2607041035) — anything on the bound port can both read every doc and query the
  chat model.

**Mitigations:**
- The failure mode is honest-refusal, not confident-wrong-answer — for a knowledge base whose whole
  point is trustworthy citations, this is the safer of the two possible failure directions.
- Sources are always listed with links back to the actual doc, so a human can always get the real
  answer by clicking through even when the model's synthesis falls short.
- If retrieval depth becomes a recurring problem: smaller chunk size (trade more chunks for finer
  granularity), or a hierarchical approach (retrieve at doc level, then re-chunk just the top document
  more finely) are the natural next things to try — not done here, flagged for later.

## Implementation

| Phase | Description | Status | Verification |
|-------|------------|--------|--------------|
| P1 | Domain + ports (`ChatAnswer`, `IEmbedder`, `IChatModel`, `IKnowledgeIndex`, `IChatHistoryStore`) | Done | code:examples/research-dashboard/src/core |
| P2 | `ollama-embedder.ts` + `ollama-chat-model.ts` secondary adapters | Done | code:examples/research-dashboard/src/adapters/secondary |
| P3 | `embedding-knowledge-index.ts` — chunking, in-memory cosine-similarity search | Done | test:`cd examples/research-dashboard && bun test` |
| P4 | Retrieval tuning: 1→2 chunks/doc, topK 5→8, after live testing exposed a real gap | Done | manual: `curl -X POST /api/chat` against a known DSpark question |
| P5 | `sqlite-chat-history-store.ts` via `bun:sqlite`, replacing an interim JSON-file design before it was ever committed | Done | code:examples/research-dashboard/src/adapters/secondary/sqlite-chat-history-store.ts |
| P6 | Chat moved to `/` (home); `/system` absorbed the displaced recent-docs widget | Done | manual: `curl /` shows chat UI, `curl /system` shows stats + recent docs |
| P7 | This ADR | Done | code:docs/adrs/ADR-2607041205-research-dashboard-rag-chat.md |

## References

- ADR-2607041035 — the base app this extends.
- Ollama endpoints used: `POST /api/embed` (`nomic-embed-text`), `POST /api/chat` (`devstral-small-2:24b`).
- `bun:sqlite` — Bun's built-in SQLite binding, no external package.
