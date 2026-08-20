# jobqueue-clean

A concurrent, durable, crash-safe job queue in a single Rust crate
(`std` + `serde`/`serde_json` only, edition 2021).

## ⚠️ Delivery semantics: AT-LEAST-ONCE (read this first)

This queue provides **at-least-once** delivery with a visibility-timeout lease.
This is a **documented, non-negotiable property — not a defect.** Exactly-once
delivery is impossible across a crash with side effects.

**What is guaranteed:**

- No two *live* workers ever both successfully `complete()` (or `fail()`) one
  job. Finalization is fenced by a monotonic `epoch` token (a "fencing token").
- Every job that ends in-flight at a crash is re-offered **exactly once per
  recovery**, while still respecting `max_attempts` and dead-lettering.
- Every value a caller observes — a `JobId`, a `Lease`, a `FailOutcome` — is on
  durable storage (fsync'd) before the call returns.

**What is NOT guaranteed:**

- That a job is *processed* at most once. A slow-but-alive worker whose lease
  expired can still be running its handler concurrently with the re-claimer.
  The internal mutex serializes *index mutation*, not *side effects*.

### Therefore: YOUR HANDLERS MUST BE IDEMPOTENT.

The recommended dedup key for your handler's side effects is
`(lease.id, lease.epoch)`. If you cannot make a side effect idempotent, you
cannot get exactly-once — no queue can.

## Usage

```rust
use std::time::Duration;
use jobqueue_clean::{Config, JobQueue, FailOutcome};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config {
        visibility_timeout: Duration::from_secs(30),
        base_backoff: Duration::from_secs(1),
        max_backoff: Duration::from_secs(300),
        default_max_attempts: 5,
        jitter: true,
        fsync_mode: jobqueue_clean::FsyncMode::PerOp,
        compact_threshold_records: 10_000,
    };

    // Recovers from ./queue-data/wal.log (+ optional snapshot.json) if present.
    let q = JobQueue::open("./queue-data", cfg)?;

    // Producer:
    let _id = q.enqueue(b"do-work")?;

    // Consumer (idempotent handler!):
    loop {
        match q.claim_blocking(Duration::from_secs(5))? {
            Some(lease) => {
                // dedup key for side effects: (lease.id, lease.epoch)
                let ok = handle(&lease.payload);
                if ok {
                    // Lost the race? complete() returns StaleLease — do NOT retry.
                    match q.complete(lease.id, lease.epoch) {
                        Ok(()) => {}
                        Err(jobqueue_clean::JobError::StaleLease) => { /* someone else owns it */ }
                        Err(e) => return Err(e.into()),
                    }
                } else {
                    match q.fail(lease.id, lease.epoch)? {
                        FailOutcome::Retrying { ready_at_ms } => {
                            let _ = ready_at_ms; // scheduled with exponential backoff
                        }
                        FailOutcome::DeadLettered => { /* exceeded max_attempts */ }
                    }
                }
            }
            None => break, // idle / timed out
        }
    }
    Ok(())
}

fn handle(_payload: &[u8]) -> bool { true }
```

## Design highlights

- **Coarse `Mutex<Inner>`** serializes the `Ready → Claimed` read-modify-write,
  giving obvious live correctness (no two live claims of one id).
- **WAL before publish, fsync before ack.** A raw `File` with an explicit
  buffer (no `BufWriter`) + `sync_data()` + directory fsync on first append.
  In-memory state is published only after the fsync succeeds (fsync-failure
  rollback).
- **Length + CRC32 framing** detects torn-write and bit-rot independent of JSON.
  A torn *trailing* frame is dropped on recovery; *interior* corruption fails
  loud with the byte offset.
- **Per-claim / per-reclaim / per-fail monotonic `epoch` fencing** so a stale
  finalize from a racing or recovered worker is rejected with `StaleLease`.
- **Crash = instant lease expiry, re-offered once** — durable, and
  dead-letterable via a `deliveries` counter so a poison message that crashes
  its worker every time still reaches the dead-letter cap.
- **Monotonic clock for live lease deadlines** so an NTP wall-clock jump cannot
  mass-expire every live lease.
- **`claim_blocking` wakes by pure clock passage** at the next due backoff or
  lease deadline — no lost timed wakeups, no mandatory reaper thread.
- **Compaction by snapshot + prefix offset** (never truncate-to-empty), so
  records appended concurrently with snapshotting are always preserved.

## Build & test

```bash
cargo build
cargo test
```

All tests are deterministic via an injected `Clock`; crash tests use a
re-exec child that `std::process::abort()`s (no destructors, no buffer flush —
a faithful crash) and a parent that reopens the directory and asserts.
