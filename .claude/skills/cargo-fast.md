---
name: cargo-fast
description: Apply and verify ADR-064 Rust compilation performance optimizations (lld linker, sccache, nextest, dev profile)
triggers:
  - speed up rust compilation
  - cargo compilation slow
  - setup cargo fast
  - cargo build slow
  - sccache setup
  - lld linker setup
  - rust compile time
---

# cargo-fast — Rust Compilation Performance Setup

**Use this skill when**: Rust builds feel slow, after a fresh clone, when setting up a new dev machine, or when adding a new contributor who hasn't run the setup.

This implements **ADR-064** (`docs/adrs/ADR-064-rust-compilation-performance.md`).

## What This Does

Runs `scripts/setup-cargo-fast.sh`, which:

1. Installs **lld** (LLVM fast linker) via brew — cuts link phase 30-50%
2. Installs **sccache** via brew — cross-build crate cache; near-instant rebuilds on branch/worktree switch
3. Installs **cargo-nextest** — parallel test execution replacing `cargo test`
4. Writes `.cargo/config.toml` with activated settings
5. Adds a `[profile.dev]` to `Cargo.toml` — deps compiled at opt-level 3, your code at 0
6. Verifies installation and shows sccache stats

## Run It

```bash
bash scripts/setup-cargo-fast.sh
```

## After Setup: Development Commands

```bash
# Build (debug, not release — dramatically faster)
cargo build -p hex-cli
cargo build -p hex-nexus

# Test (parallel, faster than cargo test)
cargo nextest run
cargo nextest run -p hex-nexus

# Check sccache effectiveness
sccache --show-stats
```

## Expected Gains (from ADR-064)

| Optimization | Phase | Improvement |
|---|---|---|
| Debug profile instead of `--release` | All | **2-5x faster** |
| lld linker | Link | **30-50% faster** |
| sccache | Compile | **Near-instant** on worktree/branch switch |
| dep opt-level 3 / code opt-level 0 | Compile | Better IDE + test perf |
| cargo-nextest | Test | **2-3x faster** test runs |

## Key Rule (ADR-064)

> **`--release` is only for CI and final artifact production.**
> All development uses debug builds (`cargo build -p <crate>` with no flags).

The release profile uses `lto = "fat"`, `codegen-units = 1`, `opt-level = "z"` — all maximally slow. Never use it for iteration.

## Diagnosing Slow Builds

```bash
# Show per-crate compile times as an HTML report
cargo build --timings -p hex-nexus

# Verify sccache is active (should show "sccache" in wrapper)
cargo build -p hex-cli -v 2>&1 | head -5

# Check cache hit rate
sccache --show-stats
```

## Re-running After Brew Updates

If brew updates LLVM, the `ld64.lld` path may change. Re-run the script to regenerate `.cargo/config.toml`:

```bash
bash scripts/setup-cargo-fast.sh
```

## ARGUMENTS

No arguments required. Run with: `/cargo-fast`
