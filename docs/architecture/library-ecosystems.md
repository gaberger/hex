# Library Ecosystem Analysis for hex-intf

> **Purpose**: Runtime reference for the `dependency-analyst` agent. Maps problem domains to optimal language + library combinations, scored on LLM generation quality, tree-sitter L2 friendliness, and production fitness.
>
> **Last updated**: 2026-03-15

---

## Scoring Legend

| Metric | Scale | Meaning |
|--------|-------|---------|
| **Maturity** | 1-5 | Community size, release cadence, documentation quality, years in production |
| **LLM Gen Quality** | 1-5 | How reliably Claude/GPT produces correct, idiomatic code. Penalizes: proc macros, implicit magic, complex generics, decorator-heavy APIs. Rewards: explicit typed APIs, clear function signatures, good training data coverage |
| **Tree-Sitter L2** | 1-5 | How well the library's API surface compresses into an L2 summary. Discrete function calls with typed params = 5. Fluent/chaining/builder APIs = 2. Macro-heavy DSLs = 1 |
| **Perf** | 1-5 | Runtime performance for the domain's typical workload |
| **Recommendation** | PRIMARY / STRONG / VIABLE / AVOID | Selection guidance |

---

## 1. Interactive Games

### Context

Game development requires tight render loops (<16ms frames), asset management, input handling, and often physics. LLM agents generating game code need libraries with explicit, stateful APIs rather than ECS macro magic.

### Comparison

| Library | Language | Maturity | LLM Gen | TS L2 | Perf | Recommendation |
|---------|----------|----------|---------|-------|------|----------------|
| **Phaser** | TypeScript | 5 | 4 | 3 | 3 | **PRIMARY** |
| **PixiJS** | TypeScript | 5 | 4 | 3 | 3 | STRONG |
| **Three.js** | TypeScript | 5 | 4 | 2 | 4 | STRONG (3D) |
| **Babylon.js** | TypeScript | 4 | 3 | 2 | 4 | VIABLE (3D) |
| **PlayCanvas** | TypeScript | 3 | 2 | 2 | 4 | VIABLE |
| **Ebitengine** | Go | 4 | 4 | 4 | 4 | **PRIMARY** |
| **Pixel** | Go | 3 | 3 | 4 | 3 | VIABLE |
| **Raylib-go** | Go | 3 | 3 | 5 | 4 | STRONG |
| **Bevy** | Rust | 4 | 2 | 1 | 5 | AVOID for LLM |
| **macroquad** | Rust | 3 | 4 | 4 | 4 | **PRIMARY** (Rust) |
| **ggez** | Rust | 3 | 3 | 3 | 4 | VIABLE |
| **wgpu** | Rust | 4 | 2 | 2 | 5 | AVOID for LLM |

### Analysis

**TypeScript / Phaser** is the default choice for 2D browser games. Phaser has the largest training corpus of any game library, meaning LLMs produce working code on the first pass reliably. The scene-based API is explicit and maps well to L2 summaries, though chained sprite configuration can compress poorly.

**Go / Ebitengine** excels for LLM-generated games due to its simple `Update() + Draw()` game loop pattern. No magic, no macros, no ECS. Functions are discrete and typed. L2 summaries capture the full API surface cleanly. Ideal when the game runs as a native binary.

**Rust / macroquad** is the only Rust game library suitable for LLM generation. Its API mimics raylib's flat C-style function calls (`draw_circle`, `is_key_pressed`), which LLMs handle well. Bevy's ECS with proc-macro queries (`Query<&Transform, With<Player>>`) produces unreliable LLM output and is nearly impossible to capture at L2 -- avoid it for agent-driven development.

### Risks

- Bevy's rapid API churn (breaking changes each minor release) compounds its LLM generation problems
- Three.js chaining patterns (`mesh.position.set(x,y,z).rotateX(a)`) compress poorly at L2
- Phaser's plugin ecosystem varies in type quality

---

## 2. Document / Slide Generation

### Context

Document generation is typically offline, batch-oriented, and template-driven. API simplicity and output quality matter more than raw performance. LLMs need to generate correct structural markup (XML for PPTX/DOCX, drawing commands for PDF).

### Comparison

| Library | Language | Maturity | LLM Gen | TS L2 | Perf | Recommendation |
|---------|----------|----------|---------|-------|------|----------------|
| **PptxGenJS** | TypeScript | 4 | 5 | 4 | 3 | **PRIMARY** (slides) |
| **pdf-lib** | TypeScript | 4 | 5 | 4 | 3 | **PRIMARY** (PDF) |
| **docx** | TypeScript | 4 | 4 | 3 | 3 | **PRIMARY** (Word) |
| **Slidev** | TypeScript | 3 | 3 | 2 | 3 | VIABLE (dev slides) |
| **reveal.js** | TypeScript | 5 | 4 | 3 | 3 | STRONG (HTML slides) |
| **go-pptx** | Go | 2 | 3 | 4 | 3 | VIABLE |
| **gofpdf** | Go | 3 | 4 | 5 | 4 | STRONG |
| **unidoc** | Go | 3 | 3 | 3 | 4 | VIABLE (commercial) |
| **printpdf** | Rust | 3 | 3 | 4 | 5 | STRONG |
| **lopdf** | Rust | 2 | 3 | 4 | 4 | VIABLE |

### Analysis

**TypeScript dominates** this domain. PptxGenJS has an explicit, object-literal API (`slide.addText("Hello", { x: 1, y: 1, fontSize: 24 })`) that LLMs generate almost perfectly. pdf-lib follows the same explicit pattern. The docx library uses a declarative tree structure that maps cleanly to L2 summaries.

**Go / gofpdf** is a solid choice for server-side PDF generation. Its imperative API (`pdf.Cell(40, 10, "Hello")`) is flat and function-call-based, scoring a perfect 5 on L2 friendliness. Limited template support compared to TypeScript options.

**Rust / printpdf** works for high-volume PDF pipelines but the ecosystem for slides and Word documents is effectively nonexistent in Rust. Not recommended as a primary choice for this domain.

### Risks

- unidoc requires a commercial license for production use
- Slidev depends on Vue's SFC format which LLMs sometimes mangle
- go-pptx has limited feature coverage compared to PptxGenJS

---

## 3. Data Processing / ETL

### Context

ETL workloads range from small CSV transforms to large-scale columnar analytics. Key concerns: throughput on large datasets, type safety for schema validation, and streaming support for memory-bounded processing.

### Comparison

| Library | Language | Maturity | LLM Gen | TS L2 | Perf | Recommendation |
|---------|----------|----------|---------|-------|------|----------------|
| **Danfo.js** | TypeScript | 3 | 3 | 2 | 2 | VIABLE |
| **Arquero** | TypeScript | 3 | 3 | 2 | 3 | VIABLE |
| **PapaParse** | TypeScript | 5 | 5 | 5 | 3 | **PRIMARY** (CSV) |
| **zod** | TypeScript | 5 | 5 | 4 | 3 | **PRIMARY** (validation) |
| **Gonum** | Go | 4 | 4 | 4 | 4 | STRONG |
| **encoding/csv** | Go | 5 | 5 | 5 | 4 | **PRIMARY** (CSV) |
| **sqlx (Go)** | Go | 4 | 4 | 4 | 4 | **PRIMARY** (SQL) |
| **GORM** | Go | 5 | 3 | 2 | 3 | VIABLE |
| **Polars** | Rust | 4 | 3 | 2 | 5 | **PRIMARY** (large data) |
| **serde** | Rust | 5 | 4 | 3 | 5 | **PRIMARY** (serialization) |
| **diesel** | Rust | 4 | 2 | 1 | 4 | AVOID for LLM |
| **sqlx (Rust)** | Rust | 4 | 3 | 3 | 4 | STRONG |

### Analysis

**TypeScript** works for small-to-medium datasets. PapaParse + zod is the highest-confidence combination for LLM generation: PapaParse for streaming CSV parsing, zod for runtime schema validation. Both have flat, explicit APIs. Danfo.js and Arquero use pandas-like chaining that compresses poorly at L2.

**Go** is the natural fit for ETL services. The standard library's `encoding/csv` combined with `sqlx` covers most pipelines with zero external dependencies. Go's explicit error handling and struct-based deserialization produce highly predictable LLM output. GORM's method chaining and implicit convention-over-configuration patterns hurt L2 summaries.

**Rust / Polars** is the performance leader for columnar analytics on large datasets (millions of rows). Its lazy evaluation API is powerful but uses method chaining that LLMs sometimes misorder. serde is universally reliable for serialization. diesel's proc-macro DSL is hostile to both LLMs and L2 summaries -- use sqlx instead.

### Risks

- Polars API changes frequently between versions; pin versions carefully
- diesel's `table!` and `#[derive(Queryable)]` macros are opaque to tree-sitter
- GORM's implicit pluralization and auto-migration can produce surprising LLM output
- Danfo.js has a small community; may stall

---

## 4. Web Frontend

### Context

Frontend frameworks are the most contested domain. Key differentiators for LLM-driven development: component model clarity, type inference quality, and how well template/JSX patterns survive L2 compression.

### Comparison

| Library | Language | Maturity | LLM Gen | TS L2 | Perf | Recommendation |
|---------|----------|----------|---------|-------|------|----------------|
| **React** | TypeScript | 5 | 5 | 3 | 3 | **PRIMARY** |
| **Svelte** | TypeScript | 4 | 4 | 2 | 4 | STRONG |
| **Vue** | TypeScript | 5 | 4 | 2 | 3 | STRONG |
| **Solid** | TypeScript | 3 | 3 | 3 | 5 | VIABLE |
| **Astro** | TypeScript | 4 | 4 | 3 | 4 | STRONG (static) |
| **syscall/js** | Go | 2 | 2 | 3 | 2 | AVOID |
| **Vugu** | Go | 1 | 1 | 2 | 2 | AVOID |
| **Yew** | Rust | 3 | 2 | 1 | 4 | AVOID for LLM |
| **Leptos** | Rust | 3 | 2 | 2 | 5 | VIABLE |
| **Dioxus** | Rust | 3 | 3 | 3 | 4 | VIABLE |
| **wasm-bindgen** | Rust | 4 | 3 | 3 | 5 | STRONG (interop) |

### Analysis

**React** has the largest LLM training corpus of any frontend framework by a wide margin. Claude and GPT generate correct React+TypeScript components with high reliability, including hooks, context, and common patterns. The JSX/TSX format does not compress ideally at L2 (template markup mixes with logic), but the function component signature model is clean enough.

**Svelte** produces excellent runtime performance and small bundles, but its `.svelte` single-file-component format with `$:` reactive declarations is a custom syntax that tree-sitter needs a dedicated parser for. LLM generation quality is good but not at React's level due to smaller training corpus.

**Go WASM** is not viable for frontend development. `syscall/js` is low-level and produces enormous WASM bundles (>5MB minimum). Vugu is effectively abandoned.

**Rust WASM** is improving rapidly. Dioxus has the most LLM-friendly API of the Rust options (React-like RSX syntax). Leptos uses signals which are clean but its `view!` macro is opaque to tree-sitter. For embedding Rust logic in a TypeScript frontend, wasm-bindgen is the correct choice -- use it as a computation bridge, not a full framework.

### Risks

- React's ecosystem fragmentation (Next vs Remix vs Vite vs CRA) confuses LLMs about project setup
- Svelte 5 runes API is new and has limited training data
- Yew's `html!` macro is deeply hostile to tree-sitter parsing
- Go WASM bundle sizes make it impractical for web

---

## 5. CLI Tools

### Context

CLI tools need fast startup, good terminal rendering, argument parsing, and interactive prompts. Binary distribution matters -- single-binary languages (Go, Rust) have an advantage over Node.js for distribution.

### Comparison

| Library | Language | Maturity | LLM Gen | TS L2 | Perf | Recommendation |
|---------|----------|----------|---------|-------|------|----------------|
| **Commander** | TypeScript | 5 | 5 | 4 | 3 | **PRIMARY** |
| **Inquirer** | TypeScript | 5 | 5 | 4 | 3 | **PRIMARY** (prompts) |
| **Chalk** | TypeScript | 5 | 5 | 5 | 3 | **PRIMARY** (color) |
| **oclif** | TypeScript | 4 | 3 | 2 | 3 | VIABLE |
| **Cobra** | Go | 5 | 5 | 5 | 5 | **PRIMARY** |
| **Bubble Tea** | Go | 4 | 4 | 3 | 5 | STRONG (TUI) |
| **Charm (Lip Gloss etc.)** | Go | 4 | 4 | 4 | 5 | STRONG (styling) |
| **Clap** | Rust | 5 | 3 | 2 | 5 | STRONG |
| **Ratatui** | Rust | 4 | 3 | 3 | 5 | STRONG (TUI) |
| **dialoguer** | Rust | 4 | 4 | 4 | 5 | STRONG (prompts) |

### Analysis

**Go / Cobra** is the best overall choice for CLI tools in LLM-driven development. Cobra's API is flat and explicit (`cmd.Flags().StringVar(&name, "name", "", "usage")`), LLMs generate it perfectly, and it compresses to clean L2 summaries. Single-binary output with zero startup latency. The Charm ecosystem (Bubble Tea for TUI, Lip Gloss for styling, Huh for forms) provides a complete toolkit.

**TypeScript / Commander + Inquirer** is the fastest to develop and has the highest LLM generation reliability. Commander's chained API is an exception to the "chaining is bad" rule because it follows a rigid, well-known pattern that every LLM has seen thousands of times. Downside: requires Node.js runtime for distribution (mitigated by `pkg` or `bun compile`).

**Rust / Clap** is powerful but its derive macro pattern (`#[derive(Parser)]` with `#[arg(...)]` attributes) is harder for LLMs to get right. The builder API alternative is more verbose but more LLM-friendly. Ratatui is excellent for rich TUIs but has a steeper learning curve. dialoguer has the cleanest API of the Rust CLI libraries.

### Risks

- oclif's class-based plugin architecture is over-engineered for most use cases and LLMs struggle with its conventions
- Bubble Tea's Elm-architecture model requires understanding `Update`/`View` patterns that some LLMs misimplement
- Clap derive macros are tree-sitter opaque

---

## 6. API Servers

### Context

HTTP API servers are the backbone of most services. Key considerations: request throughput, middleware ecosystem, type-safe routing, and how naturally the framework maps to hexagonal architecture ports.

### Comparison

| Library | Language | Maturity | LLM Gen | TS L2 | Perf | Recommendation |
|---------|----------|----------|---------|-------|------|----------------|
| **Fastify** | TypeScript | 5 | 4 | 3 | 4 | **PRIMARY** |
| **Hono** | TypeScript | 4 | 5 | 4 | 4 | **PRIMARY** |
| **tRPC** | TypeScript | 4 | 3 | 2 | 3 | STRONG (typed RPC) |
| **Express** | TypeScript | 5 | 5 | 4 | 2 | VIABLE (legacy) |
| **Chi** | Go | 5 | 5 | 5 | 5 | **PRIMARY** |
| **Fiber** | Go | 4 | 4 | 4 | 5 | STRONG |
| **Echo** | Go | 4 | 4 | 4 | 5 | STRONG |
| **net/http** | Go | 5 | 5 | 5 | 5 | STRONG (stdlib) |
| **Axum** | Rust | 4 | 3 | 3 | 5 | **PRIMARY** |
| **Actix-web** | Rust | 5 | 3 | 2 | 5 | STRONG |
| **Rocket** | Rust | 4 | 3 | 2 | 4 | VIABLE |

### Analysis

**Go / Chi** is the top recommendation for API servers. Chi composes with Go's standard `net/http` types (no framework lock-in), uses explicit middleware chaining, and has a flat routing API that both LLMs and L2 summaries handle perfectly. Go 1.22+ enhanced the standard `net/http` mux to be nearly as capable as Chi for simple routing, making the stdlib a viable zero-dependency alternative.

**TypeScript / Hono** is the modern choice for TypeScript APIs. It works across every runtime (Node, Deno, Bun, Cloudflare Workers, Lambda), has an explicit middleware API, and its router pattern is cleaner than Express. LLMs generate Hono code with high reliability. Fastify remains excellent for Node-specific deployments with its schema-based validation and plugin system.

**TypeScript / tRPC** is powerful for end-to-end type safety between frontend and backend but its generic-heavy router definition (`t.procedure.input(z.object({...})).query(...)`) compresses poorly at L2 and LLMs occasionally produce incorrect middleware chains.

**Rust / Axum** is the best Rust option. Built on tower (the standard Rust middleware ecosystem) and tokio, it uses extractors in function signatures which are moderately LLM-friendly. Actix-web's macro annotations (`#[get("/")]`) are more concise but less tree-sitter transparent.

### Risks

- Express is technically unmaintained (Express 5 has been in beta for years); use for legacy compatibility only
- tRPC's type inference chains can cause TypeScript compiler slowdowns on large projects
- Rocket requires nightly Rust features in some configurations
- Fiber's Express-like API is convenient but creates a false familiarity that can mask Go-specific patterns

---

## Cross-Domain Communication Patterns

When components use different languages, they need a communication bridge. This section maps common multi-language combinations to recommended IPC patterns, aligned with the `dependency-analyst` agent's `communication_pattern` phase.

### Pattern Selection Matrix

| Scenario | Pattern | Latency | Throughput | Complexity |
|----------|---------|---------|------------|------------|
| TS frontend + Go API | **REST/JSON** | ~5ms | Medium | Low |
| TS frontend + Rust compute | **WASM bridge** | <1ms | High | Medium |
| Go API + Rust processing | **gRPC** | ~1ms | High | Medium |
| Go API + TS worker | **WebSocket** | ~2ms | High | Medium |
| Microservices (any mix) | **NATS** | ~1ms | Very High | Medium |
| Same-machine Go + Rust | **Unix socket + protobuf** | <1ms | Very High | Low |
| Rust lib in Go binary | **FFI (C ABI)** | <0.1ms | Max | High |
| TS + Go monorepo | **tRPC or REST** | ~3ms | Medium | Low |

### Recommended Stacks by Architecture

**Monolith (single language)**: No IPC needed. Use internal port interfaces.

**Frontend + Backend**: TypeScript React + Go Chi API over REST. Use zod on the frontend and Go struct tags on the backend for schema alignment. Consider OpenAPI spec generation for contract testing.

**Compute-Intensive Frontend**: TypeScript React + Rust WASM via wasm-bindgen for heavy computation (image processing, data transformation, cryptography). Keep the WASM module focused on pure computation; let TypeScript handle DOM and state.

**High-Throughput Pipeline**: Go service ingestion + Rust Polars processing over gRPC. Protobuf contracts ensure type safety across the boundary. Use streaming RPCs for large datasets.

**Event-Driven Microservices**: NATS for inter-service messaging with any language combination. Each service is a hexagonal adapter with NATS as the communication port.

---

## Quick Reference Matrix

The primary lookup table for the `dependency-analyst` agent. For each domain, shows the recommended default configuration.

| Domain | Language | Top Library | Runner-Up | Communication | Notes |
|--------|----------|-------------|-----------|---------------|-------|
| **2D Game (browser)** | TypeScript | Phaser | PixiJS | N/A (client) | Largest LLM training corpus |
| **3D Game (browser)** | TypeScript | Three.js | Babylon.js | N/A (client) | Three.js has better LLM coverage |
| **2D Game (native)** | Go | Ebitengine | Raylib-go | N/A (binary) | Simplest game loop model |
| **2D Game (perf-critical)** | Rust | macroquad | ggez | N/A (binary) | Flat C-style API, LLM friendly |
| **Slides/PPTX** | TypeScript | PptxGenJS | reveal.js | REST from Go | Object-literal API, perfect LLM gen |
| **PDF generation** | TypeScript | pdf-lib | -- | REST from Go | Explicit drawing API |
| **PDF (high-volume)** | Go | gofpdf | -- | gRPC from Rust | Flat imperative API |
| **Word/DOCX** | TypeScript | docx | -- | REST from Go | Declarative tree structure |
| **CSV/ETL (small)** | TypeScript | PapaParse + zod | -- | N/A | Streaming + validation combo |
| **ETL service** | Go | encoding/csv + sqlx | -- | gRPC or NATS | Zero-dep, explicit error handling |
| **Analytics (large)** | Rust | Polars | -- | gRPC from Go | Columnar, lazy eval, fast |
| **Web frontend** | TypeScript | React | Svelte | REST/WS to API | Largest LLM corpus by far |
| **WASM compute** | Rust | wasm-bindgen | Dioxus | WASM bridge | Pure computation module |
| **CLI (simple)** | TypeScript | Commander | -- | N/A | Fastest to develop |
| **CLI (distributable)** | Go | Cobra | Bubble Tea (TUI) | N/A | Single binary, zero startup |
| **CLI (perf-critical)** | Rust | Clap + dialoguer | Ratatui | N/A | Use builder API, not derive |
| **API server (general)** | Go | Chi | net/http stdlib | REST/gRPC | Best L2 + LLM + perf balance |
| **API server (edge/multi-runtime)** | TypeScript | Hono | Fastify | REST | Runs everywhere |
| **API server (max throughput)** | Rust | Axum | Actix-web | gRPC | Tower middleware ecosystem |
| **Typed frontend-backend** | TypeScript | tRPC + React | -- | RPC (internal) | End-to-end type safety |

---

## LLM Generation Anti-Patterns

Libraries and patterns the `dependency-analyst` agent should steer agents away from:

| Anti-Pattern | Examples | Problem |
|-------------|----------|---------|
| **Proc macro DSLs** | Bevy ECS queries, diesel `table!`, Rocket routes | Tree-sitter cannot parse macro-expanded code; LLMs hallucinate attribute syntax |
| **Implicit convention-over-config** | GORM auto-plural, Rails-like ORMs | LLMs assume wrong conventions or mix conventions from different ORMs |
| **Deep method chaining** | Danfo.js pipelines, RxJS observables, Polars lazy chains | L2 summaries lose operation order; LLMs misorder chain steps |
| **Template DSLs** | Svelte `$:`, Vue `<script setup>`, Yew `html!` | Custom syntax requires dedicated tree-sitter grammars; training data is sparse |
| **Decorator-heavy APIs** | NestJS, TypeORM decorators, Python dataclass-style | Decorators are metadata-at-a-distance; L2 summaries miss the decorator-to-behavior mapping |
| **Implicit middleware ordering** | Express middleware stack, Actix-web guards | Order-dependent side effects are invisible in L2 summaries |

### Preferred Patterns for LLM-Driven Development

| Pattern | Examples | Why It Works |
|---------|----------|-------------|
| **Explicit function calls** | Ebitengine `Draw()`, gofpdf `Cell()`, macroquad `draw_circle()` | 1:1 mapping between code and intent; perfect L2 compression |
| **Object-literal config** | PptxGenJS `{ x: 1, fontSize: 24 }`, Hono middleware | All options visible in one place; LLMs fill in fields reliably |
| **Struct-based deserialization** | Go struct tags, serde `#[derive(Deserialize)]` | Schema is the struct; tree-sitter reads it directly |
| **Function signature extractors** | Axum extractors, Chi middleware | Types in function params; visible at L2 |
| **Builder pattern (simple)** | Clap builder, Commander `.option()` | Sequential, predictable; each call adds one thing |

---

## Version Pinning Recommendations

For reproducible LLM-generated code, pin to these known-stable versions (as of 2026-03):

| Library | Recommended Version | Reason |
|---------|-------------------|--------|
| Phaser | ^3.80 | Stable API, large corpus |
| React | ^18.x or ^19.x | 18 has most training data; 19 is current |
| Hono | ^4.x | Stable middleware API |
| Cobra | ^1.8 | Mature, stable |
| Chi | ^5.x | Compatible with Go 1.22+ mux |
| Axum | ^0.7 | Tower 0.4 compatible |
| Polars | ^0.44 | Pin tightly; API changes often |
| PptxGenJS | ^3.12 | Stable, feature-complete |
| wasm-bindgen | ^0.2 | Stable interop layer |
| Ebitengine | ^2.7 | Stable game loop API |
| macroquad | ^0.4 | Stable flat API |

---

## Decision Flowchart

```
START: What are you building?
│
├─ Runs in browser?
│  ├─ Yes → TypeScript is PRIMARY language
│  │  ├─ Game? → Phaser (2D) or Three.js (3D)
│  │  ├─ Heavy compute? → Add Rust WASM via wasm-bindgen
│  │  ├─ Static site? → Astro
│  │  └─ Interactive app? → React
│  └─ No → Continue
│
├─ Needs to be distributed as binary?
│  ├─ Yes → Go (simple) or Rust (perf-critical)
│  │  ├─ CLI? → Go Cobra (default) or Rust Clap (perf)
│  │  ├─ API server? → Go Chi (default) or Rust Axum (throughput)
│  │  └─ Game? → Go Ebitengine or Rust macroquad
│  └─ No → Continue
│
├─ Data processing / ETL?
│  ├─ < 1M rows → TypeScript (PapaParse + zod) or Go (encoding/csv + sqlx)
│  └─ > 1M rows → Rust Polars with Go or TS service layer
│
├─ Document generation?
│  └─ TypeScript always (PptxGenJS / pdf-lib / docx)
│
└─ Default → TypeScript for prototyping, Go for services, Rust for hot paths
```

---

## Hex-Intf Integration Notes

### How This Maps to Hexagonal Architecture

Each library recommendation maps to an **adapter** in the hex-intf framework:

- **Primary adapters** (CLI, HTTP, UI): Use the CLI or Web Frontend recommendations
- **Secondary adapters** (DB, file I/O, external APIs): Use the ETL/API Server recommendations
- **Domain core**: Language-agnostic; port interfaces define contracts
- **Cross-adapter communication**: Use the Communication Patterns section

### Tree-Sitter Summary Compatibility

Libraries rated 4-5 on "TS L2" produce API surfaces that compress to <200 tokens per file at L2 level, matching the hex-intf token budget architecture. Libraries rated 1-2 may require L3 (full source) loading for LLM agents to work with them correctly, which defeats the token efficiency goal.

**Rule of thumb**: If a library scores below 3 on both LLM Gen and TS L2, it should not be used in hex-intf agent-driven development regardless of other merits.
