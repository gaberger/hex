Allows searching the web for up-to-date information. Provides current information for recent events and data not in training.

CRITICAL: After answering using web search results, you MUST include a 'Sources:' section at the end with relevant URLs as markdown hyperlinks. This is MANDATORY.

## hex-specific rules

### When to use WebSearch in hex context

WebSearch is appropriate for:
- Rust crate documentation (crates.io, docs.rs) — especially for version-specific APIs
- SpacetimeDB SDK docs or changelog (if not available via context7)
- Cargo ecosystem questions (feature flags, MSRV compatibility)
- Tauri or SolidJS API updates not in training data

### Prefer these over WebSearch

| Need | Better tool |
|---|---|
| Library API docs (Rust, SolidJS, Tauri) | `mcp__plugin_context7_context7__query-docs` |
| hex codebase questions | `mcp__hex__hex_batch_search` or Grep |
| ADR content | `mcp__hex__hex_adr_search` |

### Never generate URLs

Do NOT construct or guess URLs — only use URLs provided by the user or returned by search results. Fabricated doc URLs produce 404s and waste context.

### Source attribution

Every response using WebSearch results must end with:
```
Sources:
- [Title](https://actual-url-from-results)
```
