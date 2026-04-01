Fetches content from a specified URL and processes it using AI. Takes a URL and prompt as input. Converts HTML to markdown.

Use for documentation retrieval and reading web pages. The URL must be a fully-formed valid URL.

## hex-specific rules

### When to use WebFetch

WebFetch is appropriate for:
- Fetching a specific docs.rs page for a Rust crate used in hex
- Reading a GitHub issue or PR referenced by the user
- Fetching a SpacetimeDB changelog or migration guide

### Prefer context7 for library docs

For well-known libraries (Rust std, tokio, axum, serde, tauri, solidjs), use `mcp__plugin_context7_context7__query-docs` first — it's faster and doesn't require a known URL.

### Never fabricate URLs

Only fetch URLs that:
1. The user explicitly provided in their message
2. Were returned by a prior WebSearch result
3. Are in a file you read (e.g., a link in an ADR or README)

**Do NOT construct or guess URLs.** Fabricated URLs waste context and may return incorrect content.

### Large pages

WebFetch converts HTML to markdown. For large documentation pages, provide a specific prompt to extract only the relevant section rather than processing the entire page.
