# Multi-Language Architectural Patterns

Concrete patterns for hex projects that use multiple languages simultaneously. Each pattern includes directory structure, port mapping, build orchestration, tree-sitter summary strategy, and agent assignment.

---

## Pattern 1: TypeScript Frontend + Go Backend

**When to use**: Web applications where the frontend is a SPA (React, Svelte) and the backend is a Go API server that wraps LLM and data services.

### Directory Structure

```
project/
├── frontend/                    # TypeScript (React or Svelte)
│   ├── src/
│   │   ├── ports/               # TS port interfaces (mirrors core ports)
│   │   │   ├── ILLMPort.ts      # Calls Go backend, not Claude directly
│   │   │   └── IProjectPort.ts
│   │   ├── adapters/
│   │   │   ├── api-client.ts    # HTTP/WebSocket adapter to Go backend
│   │   │   └── ui/              # UI component adapters
│   │   └── app/
│   ├── package.json
│   ├── tsconfig.json
│   └── vite.config.ts
├── backend/                     # Go API server
│   ├── cmd/
│   │   └── server/main.go
│   ├── internal/
│   │   ├── ports/               # Go port interfaces
│   │   │   ├── llm_port.go      # Wraps Claude API directly
│   │   │   ├── build_port.go
│   │   │   └── ast_port.go
│   │   ├── adapters/
│   │   │   ├── claude/          # Claude API adapter
│   │   │   ├── treesitter/      # tree-sitter Go bindings
│   │   │   └── git/
│   │   ├── domain/
│   │   └── usecases/
│   ├── go.mod
│   └── go.sum
├── shared/                      # Cross-language type definitions
│   ├── api.openapi.yaml         # OpenAPI spec (source of truth)
│   └── proto/                   # Optional: protobuf for WebSocket messages
│       └── messages.proto
├── Taskfile.yml                 # Build orchestration
├── .treesitter/                 # Summary cache (both languages)
└── tests/
    ├── e2e/                     # Playwright E2E tests
    └── contract/                # OpenAPI contract tests
```

### Port Interface Mapping

| Port | Language | Rationale |
|------|----------|-----------|
| `ILLMPort` | Go (backend) | Go server holds API keys, manages token budgets server-side |
| `ILLMPort` (frontend) | TypeScript | Thin client that calls Go `/api/llm/prompt` endpoint |
| `IASTPort` | Go (backend) | tree-sitter Go bindings are mature; parsing happens server-side |
| `IBuildPort` | Go (backend) | Orchestrates `tsc`, `go build` via subprocess |
| `IWorktreePort` | Go (backend) | Git operations run on the server filesystem |
| `IFileSystemPort` | Go (backend) | Server-side file access only |
| `ISummaryPort` | TypeScript (frontend) | Renders AST summaries for display; calls Go for extraction |

### Build Orchestration

```yaml
# Taskfile.yml
version: '3'

tasks:
  dev:
    deps: [dev:frontend, dev:backend]

  dev:frontend:
    dir: frontend
    cmd: npx vite --port 3000

  dev:backend:
    dir: backend
    cmd: go run ./cmd/server -port 8080

  build:
    cmds:
      - task: build:frontend
      - task: build:backend

  build:frontend:
    dir: frontend
    cmd: npx esbuild src/main.ts --bundle --outdir=dist --minify

  build:backend:
    dir: backend
    cmd: go build -o ../dist/server ./cmd/server

  test:
    cmds:
      - task: test:frontend
      - task: test:backend
      - task: test:e2e

  test:frontend:
    dir: frontend
    cmd: npx vitest run

  test:backend:
    dir: backend
    cmd: go test ./...

  test:e2e:
    dir: tests/e2e
    cmd: npx playwright test

  generate:types:
    desc: Generate TS types from OpenAPI spec
    cmds:
      - npx openapi-typescript shared/api.openapi.yaml -o frontend/src/generated/api.ts
      - go generate ./backend/internal/ports/...

  summarize:
    desc: Generate tree-sitter summaries for both languages
    cmds:
      - npx hex summarize --level L2 --root frontend/src --output .treesitter/frontend.txt
      - npx hex summarize --level L2 --root backend/internal --output .treesitter/backend.txt
```

### Tree-Sitter Across Language Boundaries

- Frontend summaries use `tree-sitter-typescript`; backend uses `tree-sitter-go`.
- The OpenAPI spec in `shared/api.openapi.yaml` acts as the cross-language contract.
- Tree-sitter summaries for Go port interfaces include the comment-based annotations that map to OpenAPI endpoints:

```
FILE: backend/internal/ports/llm_port.go
LANG: go
EXPORTS:
  interface LLMPort
    + Prompt(ctx context.Context, budget TokenBudget, msgs []Message) (*LLMResponse, error)
    + StreamPrompt(ctx context.Context, budget TokenBudget, msgs []Message) (<-chan string, error)
IMPORTS: [context, domain]
DEPS: none
LINES: 34
```

The corresponding frontend summary:

```
FILE: frontend/src/ports/ILLMPort.ts
LANG: typescript
EXPORTS:
  interface ILLMPort
    + prompt(budget: TokenBudget, messages: Message[]): Promise<LLMResponse>
    + streamPrompt(budget: TokenBudget, messages: Message[]): AsyncGenerator<string>
IMPORTS: [TokenBudget, Message, LLMResponse] from ../generated/api
DEPS: none
LINES: 18
```

An agent reading both summaries can verify the frontend port matches the backend port.

### Agent Assignment

| Agent | Scope | Worktree |
|-------|-------|----------|
| `ts-frontend-coder` | `frontend/src/**` | `feat/frontend-*` |
| `go-backend-coder` | `backend/internal/**` | `feat/backend-*` |
| `contract-tester` | `shared/`, `tests/contract/` | `feat/contract-*` |
| `e2e-tester` | `tests/e2e/` | `feat/e2e-*` |

One agent per language boundary. The contract-tester agent validates that OpenAPI types stay synchronized.

---

## Pattern 2: TypeScript Orchestrator + Rust Compute

**When to use**: The main application is TypeScript (Node.js or browser), but performance-critical computation (AST parsing, code generation, data processing) is implemented in Rust and called via WASM or FFI.

### Directory Structure

```
project/
├── ts/                          # TypeScript orchestrator
│   ├── src/
│   │   ├── core/
│   │   │   ├── ports/
│   │   │   │   ├── index.ts     # All port interfaces defined here
│   │   │   │   └── cross-lang.ts
│   │   │   ├── domain/
│   │   │   └── usecases/
│   │   ├── adapters/
│   │   │   ├── primary/
│   │   │   ├── secondary/
│   │   │   └── wasm-bridge/     # WASM adapter layer
│   │   │       ├── ast-bridge.ts
│   │   │       └── codegen-bridge.ts
│   │   └── infrastructure/
│   ├── package.json
│   └── tsconfig.json
├── rust/                        # Rust compute modules
│   ├── crates/
│   │   ├── ast-engine/          # tree-sitter wrapper, structural diff
│   │   │   ├── src/lib.rs
│   │   │   └── Cargo.toml
│   │   ├── codegen/             # Code generation engine
│   │   │   ├── src/lib.rs
│   │   │   └── Cargo.toml
│   │   └── wasm-api/            # WASM entry points (wasm-bindgen)
│   │       ├── src/lib.rs
│   │       └── Cargo.toml
│   ├── Cargo.toml               # Workspace root
│   └── Cargo.lock
├── napi/                        # Optional: napi-rs for Node native module
│   ├── src/lib.rs
│   ├── Cargo.toml
│   └── package.json             # Published as @project/native
├── shared/
│   └── types.schema.json        # JSON Schema for cross-language types
├── Taskfile.yml
└── tests/
    ├── unit/                    # vitest for TS, cargo test for Rust
    └── integration/             # Full pipeline tests
```

### Communication Strategies

**Option A: WASM (browser + Node)**

```typescript
// ts/src/adapters/wasm-bridge/ast-bridge.ts
import init, { extract_summary, diff_structural } from '@project/wasm-api';

export class WASMASTAdapter implements IASTPort {
  private initialized = false;

  private async ensureInit(): Promise<void> {
    if (!this.initialized) {
      await init();
      this.initialized = true;
    }
  }

  async extractSummary(filePath: string, level: ASTSummary['level']): Promise<ASTSummary> {
    await this.ensureInit();
    const raw = await fs.readFile(filePath, 'utf-8');
    const result = extract_summary(raw, filePath, level);
    return JSON.parse(result) as ASTSummary;
  }

  diffStructural(before: ASTSummary, after: ASTSummary): StructuralDiff {
    const result = diff_structural(JSON.stringify(before), JSON.stringify(after));
    return JSON.parse(result) as StructuralDiff;
  }
}
```

**Option B: napi-rs (Node only, zero-copy)**

```typescript
// Uses the native module directly — no serialization for buffers
import { extractSummary, diffStructural } from '@project/native';

export class NativeASTAdapter implements IASTPort {
  async extractSummary(filePath: string, level: ASTSummary['level']): Promise<ASTSummary> {
    return extractSummary(filePath, level);  // Direct struct mapping via napi-rs
  }

  diffStructural(before: ASTSummary, after: ASTSummary): StructuralDiff {
    return diffStructural(before, after);  // No JSON round-trip
  }
}
```

### Port Interface Mapping

| Port | Implementation Language | Communication |
|------|------------------------|---------------|
| `IASTPort` | Rust (`ast-engine` crate) | WASM or napi-rs |
| `ILLMPort` | TypeScript | Direct (TS calls Claude API) |
| `IBuildPort` | TypeScript (orchestrates subprocess) | Direct |
| `IWorktreePort` | TypeScript | Direct |
| `ICodeGenerationPort` | Rust (`codegen` crate) for template expansion, TS for LLM prompting | WASM for template engine |
| `IFileSystemPort` | TypeScript | Direct (Rust reads via passed strings) |

### Build Orchestration

```yaml
# Taskfile.yml
version: '3'

tasks:
  build:
    cmds:
      - task: build:rust-wasm
      - task: build:ts

  build:rust-wasm:
    dir: rust
    cmd: wasm-pack build crates/wasm-api --target bundler --out-dir ../../ts/src/wasm-pkg

  build:rust-native:
    dir: napi
    cmd: npm run build  # napi-rs build

  build:ts:
    dir: ts
    cmd: npx tsc && npx esbuild src/main.ts --bundle --outdir=dist

  test:
    cmds:
      - task: test:rust
      - task: test:ts
      - task: test:integration

  test:rust:
    dir: rust
    cmd: cargo test --workspace

  test:ts:
    dir: ts
    cmd: npx vitest run

  test:integration:
    dir: tests/integration
    cmd: npx vitest run

  bench:
    desc: Compare WASM vs native vs pure-TS performance
    dir: ts
    cmd: npx vitest bench
```

### Tree-Sitter Across Language Boundaries

- Rust crate APIs are summarized with `tree-sitter-rust`. The `wasm-api` crate's `#[wasm_bindgen]` exports define the cross-language contract.
- TypeScript bridge files are summarized with `tree-sitter-typescript`. An agent can read the Rust L2 summary and the TS bridge L2 summary side-by-side to verify they match.
- The JSON Schema in `shared/types.schema.json` defines the serialization contract for types that cross the WASM boundary.

### Agent Assignment

| Agent | Scope | Model Tier |
|-------|-------|------------|
| `rust-compute-coder` | `rust/crates/**` | Tier 3 (Opus) -- Rust requires careful reasoning |
| `ts-orchestrator-coder` | `ts/src/**` | Tier 2 (Sonnet) -- Standard TS orchestration |
| `wasm-bridge-coder` | `ts/src/adapters/wasm-bridge/`, `rust/crates/wasm-api/` | Tier 3 (Opus) -- Cross-language boundary |
| `integration-tester` | `tests/integration/` | Tier 2 (Sonnet) |

The `wasm-bridge-coder` agent works across both languages because the bridge is a single logical unit split across two files.

---

## Pattern 3: Go Services + Rust Libraries

**When to use**: Go handles API servers and orchestration while Rust provides performance-critical libraries for compilation, parsing, or data processing.

### Directory Structure

```
project/
├── go/                          # Go services
│   ├── cmd/
│   │   ├── api-server/main.go
│   │   └── orchestrator/main.go
│   ├── internal/
│   │   ├── ports/
│   │   │   ├── build_port.go
│   │   │   ├── ast_port.go
│   │   │   └── llm_port.go
│   │   ├── adapters/
│   │   │   ├── rustffi/         # CGo FFI to Rust libraries
│   │   │   │   ├── ast_adapter.go
│   │   │   │   └── build_adapter.go
│   │   │   ├── grpc/            # gRPC client to Rust service
│   │   │   │   └── compiler_client.go
│   │   │   └── claude/
│   │   ├── domain/
│   │   └── usecases/
│   ├── go.mod
│   └── go.sum
├── rust/                        # Rust libraries and optional gRPC service
│   ├── crates/
│   │   ├── ast-engine/
│   │   │   ├── src/lib.rs
│   │   │   └── Cargo.toml
│   │   ├── compiler-wrapper/
│   │   │   ├── src/lib.rs
│   │   │   └── Cargo.toml
│   │   └── ffi-exports/         # C ABI exports for CGo
│   │       ├── src/lib.rs       # #[no_mangle] extern "C" functions
│   │       ├── cbindgen.toml
│   │       └── Cargo.toml
│   ├── proto/                   # Shared protobuf definitions
│   │   └── compiler.proto
│   ├── Cargo.toml
│   └── Cargo.lock
├── shared/
│   └── proto/
│       ├── ast.proto
│       └── compiler.proto
├── Makefile                     # Build orchestration (Make is idiomatic for Go+Rust)
└── tests/
    ├── go-unit/
    ├── rust-unit/
    └── integration/
```

### Communication Strategies

**Option A: CGo FFI (same process, lowest latency)**

```go
// go/internal/adapters/rustffi/ast_adapter.go
package rustffi

/*
#cgo LDFLAGS: -L${SRCDIR}/../../../../rust/target/release -last_engine_ffi
#include "ast_engine_ffi.h"
*/
import "C"
import (
    "encoding/json"
    "unsafe"
)

type RustASTAdapter struct{}

func (a *RustASTAdapter) ExtractSummary(filePath string, level string) (*ASTSummary, error) {
    cPath := C.CString(filePath)
    defer C.free(unsafe.Pointer(cPath))
    cLevel := C.CString(level)
    defer C.free(unsafe.Pointer(cLevel))

    result := C.extract_summary(cPath, cLevel)
    defer C.free_string(result)

    var summary ASTSummary
    err := json.Unmarshal([]byte(C.GoString(result)), &summary)
    return &summary, err
}
```

**Option B: gRPC (separate process, scalable)**

```go
// go/internal/adapters/grpc/compiler_client.go
package grpc

type GRPCBuildAdapter struct {
    client compilerpb.CompilerServiceClient
}

func (a *GRPCBuildAdapter) Compile(project Project) (*BuildResult, error) {
    resp, err := a.client.Compile(ctx, &compilerpb.CompileRequest{
        RootPath: project.RootPath,
        Language: project.Language,
    })
    if err != nil {
        return nil, err
    }
    return toBuildResult(resp), nil
}
```

### Port Interface Mapping

| Port | Go Interface | Rust Implementation | Communication |
|------|-------------|---------------------|---------------|
| `IASTPort` | `ast_port.go` | `ast-engine` crate | CGo FFI via `ffi-exports` |
| `IBuildPort` | `build_port.go` | `compiler-wrapper` crate | gRPC (separate process) |
| `ILLMPort` | `llm_port.go` | N/A (Go handles directly) | -- |
| `IWorktreePort` | `worktree_port.go` | N/A (Go handles directly) | -- |

### Build Orchestration

```makefile
# Makefile

.PHONY: all build test clean

all: build test

# --- Rust ---
rust-lib:
	cd rust && cargo build --release

rust-ffi: rust-lib
	cd rust && cbindgen --config crates/ffi-exports/cbindgen.toml \
		--crate ffi-exports --output target/include/ast_engine_ffi.h

rust-grpc:
	cd rust && cargo build --release --bin compiler-service

# --- Go ---
go-build: rust-ffi
	cd go && CGO_LDFLAGS="-L../rust/target/release" go build ./cmd/...

go-test: rust-ffi
	cd go && CGO_LDFLAGS="-L../rust/target/release" go test ./...

# --- Combined ---
build: rust-ffi rust-grpc go-build

test: go-test
	cd rust && cargo test --workspace
	cd tests/integration && go test ./...

# --- Tree-sitter summaries ---
summarize:
	npx hex summarize --level L2 --root go/internal --output .treesitter/go.txt
	npx hex summarize --level L2 --root rust/crates --output .treesitter/rust.txt

clean:
	cd rust && cargo clean
	cd go && go clean ./...
```

### Tree-Sitter Across Language Boundaries

- Rust `#[no_mangle] extern "C"` functions in `ffi-exports` define the FFI contract. Tree-sitter summaries of this crate show the C-compatible signatures.
- Go CGo adapter files are summarized showing the `C.` function calls, letting agents verify the Go adapter matches the Rust FFI surface.
- Protobuf `.proto` files serve as the gRPC contract. Tree-sitter can parse protobuf with `tree-sitter-proto`.

### Agent Assignment

| Agent | Scope | Rationale |
|-------|-------|-----------|
| `go-service-coder` | `go/internal/**` | Go API and orchestration logic |
| `rust-lib-coder` | `rust/crates/ast-engine/`, `rust/crates/compiler-wrapper/` | Pure Rust library code |
| `ffi-bridge-coder` | `rust/crates/ffi-exports/`, `go/internal/adapters/rustffi/` | Cross-language FFI boundary |
| `proto-maintainer` | `shared/proto/`, `go/internal/adapters/grpc/` | Protobuf contract and Go gRPC client |

---

## Pattern 4: Full Polyglot (TS + Go + Rust)

**When to use**: Large systems where each language handles what it does best. TypeScript for frontend/CLI/LLM interaction, Go for API servers and orchestration, Rust for performance-critical computation.

### Directory Structure

```
project/
├── ts/                          # TypeScript: Frontend + CLI + LLM
│   ├── packages/
│   │   ├── cli/                 # CLI tool (primary adapter)
│   │   │   ├── src/
│   │   │   └── package.json
│   │   ├── web/                 # Web frontend (primary adapter)
│   │   │   ├── src/
│   │   │   └── package.json
│   │   ├── llm-client/          # LLM interaction library
│   │   │   ├── src/
│   │   │   │   ├── ports/       # ILLMPort defined here
│   │   │   │   └── adapters/    # Claude, OpenAI adapters
│   │   │   └── package.json
│   │   └── shared-types/        # Generated types from protobuf/OpenAPI
│   │       ├── src/
│   │       └── package.json
│   ├── pnpm-workspace.yaml
│   └── tsconfig.base.json
├── go/                          # Go: API server + task orchestration
│   ├── cmd/
│   │   ├── api/main.go          # HTTP/gRPC API server
│   │   └── orchestrator/main.go # Worktree + task orchestration
│   ├── internal/
│   │   ├── ports/
│   │   │   ├── worktree_port.go
│   │   │   ├── build_port.go
│   │   │   └── messaging_port.go  # NATS pub/sub
│   │   ├── adapters/
│   │   │   ├── nats/            # NATS messaging adapter
│   │   │   ├── grpc-server/     # gRPC server (called by TS)
│   │   │   ├── grpc-client/     # gRPC client (calls Rust)
│   │   │   ├── git/
│   │   │   └── worktree/
│   │   ├── domain/
│   │   └── usecases/
│   ├── go.mod
│   └── go.sum
├── rust/                        # Rust: AST parsing + codegen + perf-critical
│   ├── crates/
│   │   ├── ast-engine/          # tree-sitter wrapper
│   │   ├── codegen/             # Code generation engine
│   │   ├── analyzer/            # Static analysis
│   │   ├── grpc-service/        # gRPC server (called by Go)
│   │   └── wasm-api/            # WASM build (called by TS in browser)
│   ├── Cargo.toml
│   └── Cargo.lock
├── shared/                      # Cross-language contracts
│   ├── proto/                   # Protobuf definitions (source of truth)
│   │   ├── ast.proto
│   │   ├── build.proto
│   │   ├── codegen.proto
│   │   └── orchestration.proto
│   ├── openapi/                 # REST API spec for external consumers
│   │   └── api.v1.yaml
│   └── nats-subjects.yaml       # NATS subject hierarchy documentation
├── infra/                       # Infrastructure
│   ├── docker-compose.yml       # Local dev: NATS, services
│   ├── nats.conf
│   └── Dockerfile.*
├── Taskfile.yml                 # Top-level build orchestration
└── tests/
    ├── e2e/                     # End-to-end (Playwright for web, CLI integration)
    ├── contract/                # Cross-language contract tests
    └── load/                    # Performance / load tests
```

### Inter-Service Communication

```
                    ┌─────────────────────┐
                    │   NATS (pub/sub)     │
                    │   Event backbone     │
                    └──┬──────┬──────┬────┘
                       │      │      │
          ┌────────────┤      │      ├────────────┐
          │            │      │      │            │
    ┌─────▼─────┐ ┌───▼──────▼──┐ ┌▼────────────┐
    │ TS CLI /   │ │ Go API +     │ │ Rust gRPC   │
    │ TS Web     │ │ Orchestrator │ │ Services    │
    │            │ │              │ │             │
    │ LLM calls  │ │ Worktree mgmt│ │ AST parse   │
    │ User I/O   │ │ Task routing │ │ Codegen     │
    │            │ │ Build orch.  │ │ Analysis    │
    └─────┬─────┘ └──────┬──────┘ └─────────────┘
          │               │
          │  gRPC/REST    │  gRPC
          └───────────────┘
```

- **NATS**: Event-driven messaging for loosely coupled events (`task.created`, `build.completed`, `code.generated`). All three languages publish and subscribe.
- **gRPC**: Typed RPC for synchronous calls. TS CLI calls Go API; Go orchestrator calls Rust services.
- **WASM**: For browser-only path. TS web app loads Rust `wasm-api` for client-side AST previews.

### Port Interface Mapping

| Port | Owner Language | Consumers | Protocol |
|------|---------------|-----------|----------|
| `ILLMPort` | TypeScript | TS only | Direct (in-process) |
| `IASTPort` | Rust | Go (gRPC), TS-browser (WASM), TS-CLI (via Go) | gRPC + WASM |
| `IBuildPort` | Go | TS (gRPC) | gRPC |
| `IWorktreePort` | Go | TS (gRPC) | gRPC |
| `IGitPort` | Go | TS (gRPC) | gRPC |
| `IFileSystemPort` | Go | TS (gRPC) | gRPC |
| `ICodeGenerationPort` | Rust (template engine) + TS (LLM prompting) | Go orchestrator | gRPC + NATS events |
| `IServiceMeshPort` | Go | All | NATS subject routing |

### Build Orchestration

```yaml
# Taskfile.yml
version: '3'

vars:
  PROTO_DIR: shared/proto

tasks:
  # --- Code Generation from Protobuf ---
  proto:
    desc: Generate code from protobuf definitions for all languages
    cmds:
      - task: proto:ts
      - task: proto:go
      - task: proto:rust

  proto:ts:
    cmd: npx buf generate {{.PROTO_DIR}} --template buf.gen.ts.yaml

  proto:go:
    cmd: buf generate {{.PROTO_DIR}} --template buf.gen.go.yaml

  proto:rust:
    cmd: cd rust && cargo build --package grpc-service --features codegen

  # --- Build ---
  build:
    desc: Build all languages
    deps: [proto]
    cmds:
      - task: build:rust
      - task: build:go
      - task: build:ts

  build:rust:
    dir: rust
    cmds:
      - cargo build --release
      - wasm-pack build crates/wasm-api --target bundler --out-dir ../../ts/packages/shared-types/wasm

  build:go:
    dir: go
    cmd: go build ./cmd/...

  build:ts:
    dir: ts
    cmd: pnpm -r build

  # --- Test ---
  test:
    desc: Test all languages then integration
    cmds:
      - task: test:rust
      - task: test:go
      - task: test:ts
      - task: test:contract
      - task: test:e2e

  test:rust:
    dir: rust
    cmd: cargo test --workspace

  test:go:
    dir: go
    cmd: go test ./...

  test:ts:
    dir: ts
    cmd: pnpm -r test

  test:contract:
    desc: Verify cross-language contracts match protobuf
    dir: tests/contract
    cmd: npx vitest run

  test:e2e:
    dir: tests/e2e
    cmd: npx playwright test

  # --- Infrastructure ---
  infra:up:
    cmd: docker compose -f infra/docker-compose.yml up -d

  infra:down:
    cmd: docker compose -f infra/docker-compose.yml down

  # --- Tree-sitter summaries ---
  summarize:
    cmds:
      - npx hex summarize --level L2 --root ts/packages --output .treesitter/ts.txt
      - npx hex summarize --level L2 --root go/internal --output .treesitter/go.txt
      - npx hex summarize --level L2 --root rust/crates --output .treesitter/rust.txt

  # --- Dev (all services) ---
  dev:
    deps: [infra:up]
    cmds:
      - task: dev:rust-grpc
      - task: dev:go
      - task: dev:ts

  dev:rust-grpc:
    dir: rust
    cmd: cargo watch -x 'run --bin grpc-service'

  dev:go:
    dir: go
    cmd: go run ./cmd/api

  dev:ts:
    dir: ts
    cmd: pnpm -r dev
```

### Tree-Sitter Across All Three Languages

Each language gets its own summary file. Agents load summaries for all languages at L1, then drill into L2 for the languages they are working on.

**Cross-language contract verification**: The `contract-tester` agent loads L2 summaries from all three languages and the protobuf definitions, then verifies:

1. Every protobuf message has a corresponding type in each language.
2. Every gRPC service method has a corresponding port method.
3. NATS subject publishers have matching subscribers.

```
# .treesitter/contract-map.txt (generated)
PROTO: ast.proto::ExtractSummary
  -> rust: grpc-service/src/ast_service.rs::AstServiceImpl::extract_summary
  -> go:   internal/adapters/grpc-client/ast_client.go::GRPCASTAdapter::ExtractSummary
  -> ts:   packages/shared-types/src/ast-client.ts::ASTClient::extractSummary
STATUS: all-matched
```

### Agent Assignment

| Agent | Languages | Scope | Model Tier |
|-------|-----------|-------|------------|
| `ts-frontend-coder` | TypeScript | `ts/packages/web/**`, `ts/packages/cli/**` | Tier 2 |
| `ts-llm-coder` | TypeScript | `ts/packages/llm-client/**` | Tier 3 (LLM integration needs careful reasoning) |
| `go-api-coder` | Go | `go/internal/**` | Tier 2 |
| `go-orchestrator-coder` | Go | `go/cmd/orchestrator/**`, `go/internal/usecases/**` | Tier 3 |
| `rust-engine-coder` | Rust | `rust/crates/ast-engine/**`, `rust/crates/codegen/**` | Tier 3 |
| `rust-grpc-coder` | Rust | `rust/crates/grpc-service/**` | Tier 2 |
| `proto-maintainer` | Protobuf | `shared/proto/**` | Tier 2 |
| `bridge-coder` | TS + Rust | `rust/crates/wasm-api/**`, `ts/packages/shared-types/**` | Tier 3 |
| `contract-tester` | All | `tests/contract/**`, `.treesitter/**` | Tier 2 |
| `e2e-tester` | TypeScript | `tests/e2e/**` | Tier 2 |

**Agent coordination rule**: Agents that work on a single language operate independently in their own worktrees. Bridge agents (`bridge-coder`, `contract-tester`) run after single-language agents complete, during the integration phase.

---

## Cross-Pattern Guidelines

### When to Split vs. Keep Single Language

| Signal | Recommendation |
|--------|---------------|
| All computation fits in one language | Stay single-language |
| Need browser + server | Pattern 1 (TS + Go) or Pattern 2 (TS + Rust WASM) |
| Need 10x performance for a specific module | Pattern 2 (TS + Rust) or Pattern 3 (Go + Rust) |
| Building a platform with multiple services | Pattern 4 (full polyglot) |
| Team only knows one language | Stay single-language; add a second only when perf demands it |

### Shared Type Strategy

1. **Protobuf** (recommended for 2+ services): Define types once, generate for all languages. Works with gRPC and NATS.
2. **OpenAPI** (recommended for REST APIs): Define endpoints once, generate TS client and Go server stubs.
3. **JSON Schema** (recommended for WASM boundaries): Define types once, validate at the boundary.

Never manually duplicate type definitions across languages. Always generate from a single source of truth.

### Build Order Rule

Always build in dependency order: **Rust first, Go second, TypeScript last.**

Rust produces libraries (WASM, FFI) that Go and TS consume. Go produces gRPC servers that TS clients call. TypeScript is the outermost layer.

### Agent Boundary Rule

- **One agent per language per adapter** for single-language work.
- **One bridge agent per language boundary** for cross-language adapters (WASM bridge, FFI bridge, gRPC client/server pair).
- **One contract agent** that reads all L2 summaries and verifies cross-language consistency.
- Bridge agents always run at Tier 3 (Opus) because cross-language reasoning is inherently complex.
