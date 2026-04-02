# compile-guard

Manage the cross-session cargo compile lock to prevent build conflicts.

## Usage

```
/compile-guard [status|clear]
```

## Commands

- `/compile-guard` or `/compile-guard status` — show current lock state
- `/compile-guard clear` — force-remove a stale lock

## How it works

A lockfile at `~/.hex/compile.lock` is acquired automatically before any
`cargo build|check|test|clippy` command via PreToolUse hook, and released
via PostToolUse hook when the build finishes.

If another Claude session holds the lock, the build is **blocked** with:
```
BLOCKED: cargo compile in progress
  Session : <session-id>
  PID     : <pid>
  Since   : <timestamp>
```

The lock is automatically cleared if the holding process is dead (stale lock).

## When invoked

Run `bash hex-cli/assets/helpers/compile-guard.sh --status` to check lock state.
Run `bash hex-cli/assets/helpers/compile-guard.sh --clear` to force-clear a stale lock.
