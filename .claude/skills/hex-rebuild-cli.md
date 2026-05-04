---
name: hex-rebuild-cli
description: Rebuild hex-cli release binary and atomically replace the busy on-PATH binary without stopping running daemons
triggers:
  - rebuild hex-cli
  - rebuild hex cli
  - update hex binary
  - install new hex
  - hex cli changes
  - replace hex binary
---

# hex-rebuild-cli — Rebuild and Atomically Install hex-cli

**Use this skill when**: you've edited `hex-cli/src/**` and need the new binary on PATH. Distinct from `/hex-dev-rebuild`, which targets `hex-nexus` (the daemon). This is for the user-facing `hex` CLI.

The catch: the sched daemon, hooks, and any open shell are likely already executing the on-PATH `hex`, so a naive `cp` fails with `Text file busy`. The fix is to write to a sibling path and `mv` (rename) — the rename atomically swaps the inode, the running process keeps its old inode mapping, and the next `hex` invocation picks up the new binary.

## Step 1 — Build

```bash
cargo build -p hex-cli --release
```

If `cargo` isn't on PATH (some shells don't source `~/.cargo/env`), use the absolute path:

```bash
~/.cargo/bin/cargo build -p hex-cli --release
```

If the build fails, STOP and report. Do not proceed to install a stale binary.

## Step 2 — Locate the on-PATH hex

```bash
which hex
```

Common locations: `/home/<user>/.local/bin/hex`, `/usr/local/bin/hex`, `~/.cargo/bin/hex`. Capture the path; you'll need it in step 3.

## Step 3 — Atomic replace

```bash
cp target/release/hex <PATH-FROM-STEP-2>.new
mv <PATH-FROM-STEP-2>.new <PATH-FROM-STEP-2>
```

**Why two steps**: `cp` directly to the busy target fails with `Text file busy` on Linux. Writing to `.new` first and then `mv` works because `mv` (rename) on the same filesystem unlinks the old inode and creates a new directory entry pointing at the new inode in one syscall. The running process keeps executing the unlinked inode until it exits; new `hex` invocations get the new binary.

**Same filesystem requirement**: `mv` is only atomic when source and destination are on the same filesystem. If `target/release/` and `~/.local/bin/` are on different mounts, copy to a temp file *next to* the destination first.

## Step 4 — Verify

```bash
hex --version
```

The version string should reflect the new build. If the daemon is using sched-related changes, also restart it so the daemon picks up the new binary:

```bash
hex sched daemon-restart
```

## Step 5 — Sanity check

If your changes touched a specific subcommand, exercise it:

```bash
hex <changed-subcommand> --help
```

## Common pitfalls

- **`cp` directly** — fails with `Text file busy` whenever a daemon, hook, or shell is running `hex`. Always use `cp .new` + `mv`.
- **Forgetting daemon-restart** — the long-running sched daemon keeps the OLD binary loaded in memory until restart. CLI invocations get the new binary, but daemon-side behavior (hook routing, tick logic) stays old.
- **Cross-filesystem mv** — silent fallback to copy-then-delete loses atomicity and can fail mid-way leaving no `hex` on PATH. Stage the temp file on the destination filesystem.
- **Building debug instead of release** — `cargo build -p hex-cli` produces `target/debug/hex`, not `target/release/hex`. The on-PATH binary is the release build.

## Why not `hex doctor self-update`

`hex doctor` doesn't currently have a self-update flow. If/when it does, prefer it — but the atomic-rename pattern stays correct as the underlying mechanism.

## ARGUMENTS

No arguments required. Run with: `/hex-rebuild-cli`
