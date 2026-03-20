# ADR-038: Vite for Development, Axum for Production

- **Status**: Proposed
- **Date**: 2026-03-20
- **Informed by**: ADR-036, current hex-chat implementation
- **Authors**: Gary

## Context

Currently, `hex-chat web` serves a statically embedded HTML file via Axum using `include_str!()`. This approach:
- Works for production (single binary deployment)
- Slows development iteration (recompile Rust on every HTML/CSS change)
- Makes frontend development cumbersome (no hot module replacement)

We want faster frontend iteration during development while maintaining a Rust-powered production server.

## Decision

### Development Mode (Vite)

Create a `hex-chat/ui/` directory with a standard Vite + TypeScript setup:

```
hex-chat/ui/
  index.html
  src/
    main.ts
    App.tsx
    styles.css
  vite.config.ts
  package.json
```

Run the frontend independently during development:
```bash
cd hex-chat/ui && npm run dev    # Vite dev server on port 5173
hex nexus start                   # hex-nexus + hex-chat web on port 5556
```

The Vite dev server proxies API calls to hex-nexus.

### Production Mode (Axum)

In production, Axum serves the built assets from `hex-chat/ui/dist/`:

```rust
// hex-chat/src/web.rs
use axum::{
    Router,
    routing::get,
    response::Html,
};
use tower_http::services::ServeDir;

pub async fn run(port: u16, nexus_url: String) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(index))
        .route("/assets/*path", get(serve_assets))
        .with_state(WebState { nexus_url });

    // ... rest unchanged
}

async fn serve_assets(
    State(state): State<WebState>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> impl IntoResponse {
    let file_path = format!("hex-chat/ui/dist/assets/{}", path);
    tokio::fs::read(&file_path)
        .await
        .map(|content| {
            let mime = match file_path.rsplit('.').next() {
                Some("js") => "application/javascript",
                Some("css") => "text/css",
                Some("html") => "text/html",
                _ => "application/octet-stream",
            };
            (StatusCode::OK, [(header::CONTENT_TYPE, mime)], content)
        })
        .unwrap_or_else(|_| (StatusCode::NOT_FOUND, [], "Not found".as_bytes()))
}
```

### Build Pipeline

```bash
# Build frontend
cd hex-chat/ui && npm run build   # Output: dist/

# Build Rust binary (includes dist/ at compile time)
cargo build -p hex-chat --release
```

For production, embed `dist/` using `rust-embed` or include_bytes!():
```rust
const INDEX_HTML: &str = include_str!("../ui/dist/index.html");
```

### Development vs Production

| Aspect | Development | Production |
|--------|-------------|------------|
| Frontend | Vite dev server (hot reload) | Served by Axum from dist/ |
| Port | 5173 (Vite) | 5556 (hex-chat web) |
| API proxy | Vite proxy → hex-nexus | Axum → hex-nexus |
| Rebuild needed | No (Vite HMR) | Yes (rebuild Rust) |

### Fallback

If `hex-chat/ui/dist/` doesn't exist at runtime, Axum falls back to serving a minimal embedded HTML:
```rust
async fn index(...) -> Html<String> {
    let dist_path = "hex-chat/ui/dist/index.html";
    match tokio::fs::read_to_string(dist_path).await {
        Ok(html) => Html(html),
        Err(_) => Html(EMBEDDED_FALLBACK.into()),
    }
}
```

## Consequences

### Positive
- Frontend iteration: 10s → 0.5s (HMR)
- Standard tooling: Use Vite plugins, TypeScript, React/solid/preact
- Production: Single Rust binary, no Node.js runtime needed
- Best of both worlds: fast dev, lean prod

### Negative
- Two build steps in production (npm build + cargo build)
- More complex project structure
- Need to ensure dev/prod parity (Vite proxy behavior matches Axum)

### Risks
- Dev/prod differences: ensure Vite dev API proxy mirrors production Axum routes
- Build artifacts: dist/ must be gitignored, but included in npm package
