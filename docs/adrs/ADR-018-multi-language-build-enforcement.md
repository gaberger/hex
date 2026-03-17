# ADR-018: Multi-Language Build Enforcement (Go + Rust)

| Field      | Value                        |
|------------|------------------------------|
| Status     | Accepted                     |
| Date       | 2026-03-17                   |
| Deciders   | Gary, Claude                 |
| Supersedes | —                            |
| Relates to | ADR-003 (Multi-Language), ADR-005 (Quality Gates) |

## Context

hex has first-class multi-language support for TypeScript, Go, and Rust at the **analysis layer** (tree-sitter parsing, boundary checking, dead export detection, layer classification). However, the **enforcement layer** — build, lint, test, CI, and pre-commit — only invokes TypeScript toolchains (`tsc`, `eslint`, `bun test`).

A 2026-03-17 investigation found that ~1 in 10 commits is a follow-up fix for incomplete code generation. The #1 blind spot is Rust (hex-hub) code that compiles but is never checked in CI. Coordination routes existed as dead code for multiple commits because no gate caught the missing module registration.

## Decision

Extend `IBuildPort` / `BuildAdapter` to dispatch compile, lint, and test commands based on the configured `Language`. Add Rust and Go build steps to CI. Update the pre-commit hook to check all detected languages.

### Build Adapter Dispatch

| Method    | TypeScript          | Go                    | Rust                    |
|-----------|--------------------|-----------------------|-------------------------|
| compile() | `tsc --noEmit`     | `go build ./...`      | `cargo check`           |
| lint()    | `eslint --format json .` | `golangci-lint run --out-format json` | `cargo clippy -- -D warnings` |
| test()    | `bun test <files>` | `go test ./... -json` | `cargo test`            |

### CI Pipeline

Add a `rust-check` job that:
1. Installs Rust toolchain
2. Runs `cargo check`, `cargo clippy`, `cargo test` for hex-hub
3. Builds the release binary and verifies `--build-hash` works

### Pre-commit Hook

Detect languages present in staged files and run the corresponding toolchain checks.

## Consequences

- **Positive**: Rust and Go compilation errors caught before commit/push, not after manual `hex analyze`
- **Positive**: Unregistered modules caught at `cargo check` time (unused import warnings become errors with `-D warnings`)
- **Negative**: Pre-commit hook becomes slower when Rust files are staged (~5-10s for `cargo check`)
- **Mitigated**: Only run Rust/Go checks when those file types are in the staged changeset
