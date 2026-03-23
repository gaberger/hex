# ADR-064: Rust Compilation Performance

**Status:** Accepted
**Date:** 2026-03-22
**Drivers:** hex-intf has a 6-crate Rust workspace (hex-cli, hex-nexus, hex-core, hex-parser, hex-agent, hex-desktop) plus WASM modules. The release profile uses `lto = "fat"`, `codegen-units = 1`, and `opt-level = "z"` — all maximally slow. Development iteration suffers when `--release` is used habitually.

## Context

Clean release builds of hex-nexus (the heaviest crate, with `git2`, `rusqlite`, `russh`, `spacetimedb-sdk`, `axum`, `reqwest`) take minutes due to the aggressive release profile. Contributors often build with `--release` because CLAUDE.md documents release commands, leading to unnecessary wait times during iteration.

### Current Release Profile

```toml
[profile.release]
opt-level = "z"      # optimize for size — slowest optimization pass
lto = "fat"          # whole-program link-time optimization — extremely slow
codegen-units = 1    # serialize all codegen — no parallelism
strip = true
panic = "abort"
```

This profile is correct for **final artifact production** (smallest binary, best runtime perf) but is the worst possible choice for development builds.

### Build Environment

- **CPU:** 12 cores (Apple Silicon)
- **Linker:** Default macOS `ld` (single-threaded)
- **Caching:** None (no sccache)
- **Codegen backend:** Default LLVM

## Decision

### 1. Reserve `--release` for CI and final builds only

Day-to-day development uses `cargo build -p hex-cli` (debug profile). The debug profile uses `opt-level = 0`, 12 codegen units (parallel), no LTO, and no stripping.

### 2. Configure `.cargo/config.toml` for build performance

```toml
[build]
jobs = 12                    # saturate all cores
rustc-wrapper = "sccache"   # cross-build crate caching
```

### 3. Use a fast linker (lld)

Linking is 30-50% of debug build wall time. The default macOS `ld` is single-threaded. LLVM's `ld64.lld` parallelizes across all cores:

```toml
[target.aarch64-apple-darwin]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=/opt/homebrew/opt/llvm/bin/ld64.lld"]
```

**Prerequisite:** `brew install llvm`

### 4. Use sccache for cross-build caching

sccache caches compiled crate artifacts across `cargo clean`, branch switches, and worktree checkouts. This is especially valuable with hex's worktree-based feature development workflow, where the same crates are compiled repeatedly across isolated worktrees.

**Prerequisite:** `brew install sccache`

### 5. Disable default features on reqwest

All four crates using `reqwest` should specify `default-features = false` to avoid compiling unused TLS backends:

```toml
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
```

This eliminates the OpenSSL/native-tls compilation path, reducing the dependency tree.

### 6. Future: Cranelift codegen backend (experimental)

When stabilized, the Cranelift backend can replace LLVM for dev builds, cutting codegen time 30-50%:

```toml
[unstable]
codegen-backend = true

[profile.dev]
codegen-backend = "cranelift"
```

Requires nightly. **Not adopted now** — revisit when stable.

## Setup Checklist

```bash
# 1. Install fast linker
brew install llvm

# 2. Install sccache
brew install sccache

# 3. Uncomment the relevant lines in .cargo/config.toml
#    (linker, rustflags, rustc-wrapper)

# 4. Verify
cargo build -p hex-cli 2>&1 | tail -1   # Should use sccache + lld
sccache --show-stats                      # Verify cache hits
```

## Expected Impact

| Change | Build Phase | Estimated Improvement |
|--------|------------|----------------------|
| Drop `--release` for dev | All | **2-5x faster** (no LTO, parallel codegen) |
| lld linker | Link | **30-50% link time reduction** |
| sccache | Compile | **Near-instant rebuilds on branch switch** |
| reqwest `default-features = false` | Compile | **10-15% fewer crates** |
| Cranelift (future) | Codegen | **30-50% codegen reduction** |

## Consequences

- **Positive:** Dramatically faster iteration, especially for multi-worktree workflows
- **Positive:** sccache benefits all contributors, not just the original builder
- **Negative:** Two new brew dependencies (llvm, sccache) — documented in setup checklist
- **Negative:** Debug builds produce larger, slower binaries — acceptable for development
- **Unchanged:** Release profile remains maximally optimized for production artifacts
- **Unchanged:** CI continues using `--release` for published binaries

## References

- [The Rust Performance Book — Compile Times](https://nnethercote.github.io/perf-book/compile-times.html)
- [sccache](https://github.com/mozilla/sccache)
- [Cranelift backend tracking issue](https://github.com/rust-lang/rust/issues/77369)
