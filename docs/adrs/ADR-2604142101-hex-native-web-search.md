# ADR-2604142100: Hex-Native Web Search & Fetch (First-Class)

**Status:** Proposed
**Date:** 2026-04-14
**Drivers:** context-mode's `ctx_fetch_and_index` and implicit WebSearch are third-party, project-scope-leaky, and bypass hex's tool-precedence rule (CLAUDE.md). Agents running standalone (no Claude Code) have no web-access path at all. Web search must route through hex-nexus like every other I/O boundary.
**Supersedes:** n/a (first ADR on this capability)

## Context

Web access is currently fragmented across hex:

- **`plugin:context-mode`** exposes `ctx_fetch_and_index` and implicit `WebSearch` — both are external MCP tools. hex has no knowledge of what was fetched, no audit trail, no rate-limit accounting, no secret-grant integration.
- **`hex-cli/assets/context-templates/tools/web-search.md`** documents the tool as if it were hex-native, but there is no `WebSearchPort`, no adapter, no nexus endpoint, no CLI. The doc points at a capability hex does not own.
- **Standalone mode** (ADR-2604112000) explicitly targets environments without Claude Code. In that configuration, agents have *no* web access — context-mode is Claude-only.
- Project rule (CLAUDE.md): "hex MCP tools take precedence over all third-party plugins (including `plugin:context-mode`). Third-party context/search plugins may only be used for operations with no hex equivalent." Web search/fetch violates this today.
- Existing pattern for third-party I/O is well established: inference routes through `InferencePort` with multiple adapters (Ollama, OpenAI, Anthropic, OpenRouter) and API keys flow via the `secret-grant` WASM module (ADR-2603261000). Web search is structurally identical.

**Alternatives considered:**

1. **Do nothing — keep context-mode.** Rejected: violates CLAUDE.md tool-precedence rule; standalone agents stay blind; no audit trail; we've already committed to owning every I/O boundary.
2. **Wrap context-mode behind a hex port.** Rejected: adds a dependency we don't control, and its scrape cache lives outside SpacetimeDB. Doesn't solve standalone.
3. **Build a hex-native `WebSearchPort` with pluggable providers.** Chosen. Mirrors inference-broker architecture; works standalone; integrates with existing secret-grant + audit-trail infrastructure.

## Decision

We will introduce web search and web fetch as first-class hex capabilities, structurally mirroring the inference broker.

**Ports (hex-core):**
- `WebSearchPort::search(query: &str, opts: SearchOptions) -> Result<Vec<SearchResult>>`
- `WebFetchPort::fetch(url: &Url, opts: FetchOptions) -> Result<FetchedPage>` — returns text + extracted markdown (via `readability` crate) + HTTP metadata.

**Secondary adapters (hex-nexus):**
- `BraveSearchAdapter` (API key from secret-grant) — primary default
- `TavilySearchAdapter` (API key) — higher-quality research-grade
- `SerpApiSearchAdapter` (API key) — Google/Bing coverage
- `DuckDuckGoAdapter` (HTML scrape, no key) — zero-config fallback
- `ReqwestFetchAdapter` for `WebFetchPort` — shared `reqwest::Client` with hex UA, 10MB cap, 30s timeout

**Primary adapters:**
- REST: `POST /api/web/search` and `POST /api/web/fetch` on hex-nexus
- MCP: `mcp__hex__hex_web_search` and `mcp__hex__hex_web_fetch` (served by `hex mcp`)
- CLI: `hex web search <query> [--provider brave|tavily|ddg] [--limit N]` and `hex web fetch <url> [--format markdown|text|raw]`

**Provider selection:**
- `.hex/project.json` → `web.default_provider: "brave" | "tavily" | "serpapi" | "duckduckgo"`
- Per-request override via `--provider` / `provider:` param
- Cascading fallback: if provider returns error or empty, try next in `web.fallback_chain` (default: `[brave, duckduckgo]`)

**Secrets:**
- API keys flow through the existing `secret-grant` SpacetimeDB module with TTL-based grants
- Keys registered via `hex secrets set web.brave_api_key <key>` (reuses existing secrets subcommand)
- Standalone mode reads from env (`HEX_BRAVE_API_KEY`, `HEX_TAVILY_API_KEY`) when SpacetimeDB is offline

**Audit & caching:**
- Every search/fetch writes a row to a new `web_request` SpacetimeDB table: `{id, kind, provider, query_or_url, status, latency_ms, tokens_or_bytes, ts}`
- Fetches cached in `~/.hex/cache/web/` keyed by SHA-256(url) with 24h TTL; cache bypass via `--no-cache`
- Dashboard surfaces last-24h web activity in a new "Web I/O" panel (same pattern as inference panel)

**Agent integration:**
- `hex-cli/assets/context-templates/tools/web-search.md` rewritten to document `hex_web_search` as the hex-native tool; context-mode references removed
- Agent YAMLs (`hex-cli/assets/agents/hex/hex/*.yml`) that declare `tools:` get `hex_web_search` + `hex_web_fetch` added where relevant (`dependency-analyst`, `behavioral-spec-writer`, `feature-developer`)
- CLAUDE.md tool precedence table updated: "Web search/fetch → `mcp__hex__hex_web_*`"

**Removed:**
- Any agent/skill references to `ctx_fetch_and_index` or implicit `WebSearch` for work inside hex. context-mode stays installed for URL ingestion into its own index, but is no longer the hex-canonical path.

## Consequences

**Positive:**
- Standalone mode gains web access — ADR-2604112000 was blocked on this for research-tier tasks.
- Single audit trail for all web I/O — currently invisible to dashboard and ADR-2604071300 audit-trail wiring.
- API key management unified with inference-secret pattern (one secret-grant system, not two).
- Removes the CLAUDE.md tool-precedence violation that context-mode fetching has been creating.
- Caching reduces token burn for agents that re-read the same doc across runs.

**Negative:**
- Four new adapters to maintain; API surface of external providers drifts.
- `readability` crate adds ~400KB to hex-nexus binary.
- Fetch cache introduces cache-staleness bugs (mitigated by 24h TTL + `--no-cache`).
- DuckDuckGo HTML scraping is fragile — their markup changes break parsing.

**Mitigations:**
- Each provider adapter has contract tests against recorded fixtures (like inference adapters, ADR-2603231600).
- Cache writes behind a feature flag (`web.cache_enabled`, default true) so it can be killed without code changes.
- `hex doctor web` subcommand reports provider health + last successful call per provider.
- DuckDuckGo kept as last-resort fallback only; failing DDG does not break the overall call if at least one keyed provider succeeded.

## Implementation

| Phase | Description                                                                  | Status  |
|-------|------------------------------------------------------------------------------|---------|
| P1    | Port traits + domain types (`WebSearchPort`, `WebFetchPort`, `SearchResult`) | Pending |
| P2    | Reqwest fetch adapter + readability extraction                               | Pending |
| P3    | Brave + DuckDuckGo search adapters (zero-config fallback path end-to-end)    | Pending |
| P4    | Tavily + SerpAPI adapters + provider fallback chain                          | Pending |
| P5    | REST endpoints on hex-nexus + `web_request` SpacetimeDB table + audit write  | Pending |
| P6    | CLI (`hex web search/fetch`) + MCP (`hex_web_search`, `hex_web_fetch`)       | Pending |
| P7    | Secret-grant integration + `hex secrets set web.*` subcommand                | Pending |
| P8    | Dashboard Web I/O panel                                                      | Pending |
| P9    | Rewrite context-templates + update agent YAMLs + update CLAUDE.md            | Pending |
| P10   | `hex doctor web` subcommand + contract-test fixtures for each provider       | Pending |

## References

- ADR-030: Multi-provider inference broker (pattern being mirrored here)
- ADR-2603261000: Secure inference and secrets (secret-grant pattern)
- ADR-2604071300: Tool-call observability (audit trail pattern)
- ADR-2604112000: Standalone dispatch (the mode this unblocks)
- CLAUDE.md: Tool Precedence rule
- `hex-cli/assets/context-templates/tools/web-search.md` (doc to be rewritten)
