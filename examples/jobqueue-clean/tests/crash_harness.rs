//! Crash-recovery tests using a re-exec child that performs scripted operations
//! and then `std::process::abort()`s (abort = no destructors, no buffer flush —
//! a faithful crash). The parent reopens the dir and asserts.
//!
//! The child entry point is dispatched at the top of each test via the
//! `JQ_CHILD` env var. When `JQ_CHILD` is set, the test binary runs the child
//! routine and aborts instead of running the normal test harness — so we gate
//! that in a constructor-like check at the start of every test in this file.

mod common;

use std::process::Command;

use common::{test_cfg, TempDir};
use jobqueue_clean::{FailOutcome, JobError, JobQueue, ManualClock};

/// Dispatch the child routine if JQ_CHILD is set. Call this at the start of
/// every test in this file. Returns true if we are the parent (continue test).
fn child_dispatch() {
    let scenario = match std::env::var("JQ_CHILD") {
        Ok(s) => s,
        Err(_) => return,
    };
    let dir = std::env::var("JQ_DIR").unwrap();
    let clock = ManualClock::new(1_000_000);
    let cfg = test_cfg();
    let q = JobQueue::open_with_clock(&dir, cfg, clock.clone()).unwrap();

    match scenario.as_str() {
        "claim_then_abort" => {
            // Enqueue A,B,C; claim one; abort before completing.
            let _a = q.enqueue(b"A").unwrap();
            let _b = q.enqueue(b"B").unwrap();
            let _c = q.enqueue(b"C").unwrap();
            let lease = q.claim().unwrap().unwrap();
            // Record the claimed id + epoch for the parent.
            println!("CLAIMED {} {}", lease.id, lease.epoch);
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
            std::process::abort();
        }
        "complete_then_abort" => {
            let a = q.enqueue(b"A").unwrap();
            let lease = q.claim().unwrap().unwrap();
            assert_eq!(lease.id, a);
            q.complete(lease.id, lease.epoch).unwrap();
            println!("DONE {}", a);
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
            std::process::abort();
        }
        "poison_claim_abort" => {
            // Single job, claim (incrementing deliveries durably), then abort.
            let a = q.enqueue_with_max(b"poison", 3).unwrap();
            let lease = q.claim().unwrap().unwrap();
            assert_eq!(lease.id, a);
            println!("CLAIMED {} {}", lease.id, lease.epoch);
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
            std::process::abort();
        }
        "compaction_concurrent" => {
            // Enqueue a few, force compaction, then append more, then abort.
            let _ = q.enqueue(b"j0").unwrap();
            let j1 = q.enqueue(b"j1").unwrap();
            let lease = q.claim().unwrap().unwrap();
            q.complete(lease.id, lease.epoch).unwrap();
            // compact now (captures snap_offset internally)
            q.compact_now().unwrap();
            // append more records AFTER snapshot
            let _ = q.enqueue(b"j2").unwrap();
            let _ = q.enqueue(b"j3").unwrap();
            let _ = j1;
            println!("PRECOMPACT_DONE");
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
            std::process::abort();
        }
        "dir_fsync_first_append" => {
            let a = q.enqueue(b"durable-one").unwrap();
            println!("ENQ {}", a);
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
            std::process::abort();
        }
        other => panic!("unknown child scenario {other}"),
    }
}

/// Run the child scenario in a subprocess; returns its stdout.
///
/// We invoke the dedicated `__child_entry` test by exact name and pass the
/// scenario via env. That test calls `child_dispatch()`, which performs the
/// scripted ops and aborts before returning.
fn run_child(scenario: &str, dir: &std::path::Path) -> String {
    let exe = std::env::current_exe().unwrap();
    let out = Command::new(exe)
        .arg("--exact")
        .arg("__child_entry")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env("JQ_CHILD", scenario)
        .env("JQ_DIR", dir)
        .output()
        .expect("spawn child");
    let mut s = String::from_utf8_lossy(&out.stdout).to_string();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}

/// Find a line containing `marker` and return the whitespace tokens that follow
/// it on that line. Robust to the test runner prefixing the line.
fn extract_after(out: &str, marker: &str) -> Option<Vec<String>> {
    for line in out.lines() {
        if let Some(pos) = line.find(marker) {
            let rest = &line[pos + marker.len()..];
            let toks: Vec<String> = rest.split_whitespace().map(|s| s.to_string()).collect();
            return Some(toks);
        }
    }
    None
}

/// The child process always runs this test. When JQ_CHILD is unset (parent
/// process running its own suite), it is a no-op.
#[test]
fn __child_entry() {
    child_dispatch();
    // If we get here, JQ_CHILD was unset → nothing to do.
}

// Helper extension used only in child routines.
trait EnqMax {
    fn enqueue_with_max(&self, payload: &[u8], max: u32) -> Result<u64, JobError>;
}
impl EnqMax for JobQueue {
    fn enqueue_with_max(&self, payload: &[u8], max: u32) -> Result<u64, JobError> {
        use jobqueue_clean::EnqueueOpts;
        self.enqueue_opts(
            payload,
            EnqueueOpts {
                max_attempts: Some(max),
                ..Default::default()
            },
        )
    }
}

#[test]
fn test_crash_recovery_claimed_reoffered_once() {
    child_dispatch();
    let tmp = TempDir::new();
    let out = run_child("claim_then_abort", tmp.path());
    let (claimed_id, old_epoch) = {
        let toks = extract_after(&out, "CLAIMED").expect("child should print CLAIMED");
        (
            toks[0].parse::<u64>().unwrap(),
            toks[1].parse::<u64>().unwrap(),
        )
    };

    // Reopen.
    let clock = ManualClock::new(2_000_000);
    let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();

    // The claimed job is Ready again and re-claimable exactly once.
    // First, completing it under the ORIGINAL epoch must fail (StaleLease).
    let r = q.complete(claimed_id, old_epoch);
    assert!(matches!(r, Err(JobError::StaleLease)), "old epoch must be stale, got {r:?}");

    // Re-claim it; the new epoch succeeds.
    // Claim until we get the previously-claimed id.
    let mut got = None;
    for _ in 0..10 {
        if let Some(lease) = q.claim().unwrap() {
            if lease.id == claimed_id {
                got = Some(lease);
                break;
            }
        } else {
            break;
        }
    }
    let lease = got.expect("re-claim of the crashed job");
    assert_ne!(lease.epoch, old_epoch, "epoch must have advanced");
    q.complete(lease.id, lease.epoch).expect("new epoch completes");
    assert!(q.is_done(claimed_id));
}

#[test]
fn test_completed_not_reoffered() {
    child_dispatch();
    let tmp = TempDir::new();
    let out = run_child("complete_then_abort", tmp.path());
    let id: u64 = extract_after(&out, "DONE").expect("DONE")[0]
        .parse()
        .unwrap();

    let clock = ManualClock::new(2_000_000);
    let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
    assert!(q.is_done(id), "completed job must remain Done after crash");
    // Never re-offered.
    let mut claims = Vec::new();
    while let Some(lease) = q.claim().unwrap() {
        claims.push(lease.id);
        q.complete_or_ignore(lease.id, lease.epoch);
    }
    assert!(!claims.contains(&id), "done job must not be re-offered");
}

#[test]
fn test_dead_letter_via_deliveries() {
    child_dispatch();
    let tmp = TempDir::new();
    // Three crash deliveries with max_attempts=3 → dead on the 3rd delivery cap.
    let mut last_id = None;
    for _ in 0..3 {
        let out = run_child("poison_claim_abort", tmp.path());
        if let Some(toks) = extract_after(&out, "CLAIMED") {
            last_id = Some(toks[0].parse::<u64>().unwrap());
        }
        // Reopen between crashes happens implicitly inside the child each time.
    }
    let id = last_id.expect("a claimed id");

    // Reopen and tick — the job should be Dead (deliveries cap) and never
    // re-offered.
    let clock = ManualClock::new(9_000_000);
    let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
    q.tick().unwrap();
    // The recovery normalization should have dead-lettered it OR the next claim
    // would dead-letter via the deliveries cap. Force a claim attempt.
    let claimed = q.claim().unwrap();
    assert!(
        claimed.is_none() || claimed.unwrap().id != id,
        "poison job must not be handed out again"
    );
    assert!(q.is_dead(id), "poison job must be Dead after delivery cap");
}

#[test]
fn test_compaction_preserves_concurrent_writes() {
    child_dispatch();
    let tmp = TempDir::new();
    let out = run_child("compaction_concurrent", tmp.path());
    assert!(out.contains("PRECOMPACT_DONE"), "child ran compaction path");

    let clock = ManualClock::new(2_000_000);
    let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
    let s = q.stats();
    // j0, j1(done), j2, j3 => 4 total; one done, three ready.
    assert_eq!(s.total, 4, "all records incl. concurrent ones recovered: {s:?}");
    assert_eq!(s.done, 1, "the completed job survived");
    assert_eq!(s.ready, 3, "the post-snapshot enqueues survived");
}

#[test]
fn test_dir_fsync_path() {
    child_dispatch();
    let tmp = TempDir::new();
    let out = run_child("dir_fsync_first_append", tmp.path());
    let id: u64 = extract_after(&out, "ENQ").expect("ENQ")[0]
        .parse()
        .unwrap();
    let clock = ManualClock::new(2_000_000);
    let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
    let s = q.stats();
    assert_eq!(s.total, 1, "the durable enqueue survived the crash");
    assert_eq!(s.ready, 1);
    let _ = id;
}

// Small convenience used in test_completed_not_reoffered.
trait CompleteOrIgnore {
    fn complete_or_ignore(&self, id: u64, epoch: u64);
}
impl CompleteOrIgnore for JobQueue {
    fn complete_or_ignore(&self, id: u64, epoch: u64) {
        let _ = self.complete(id, epoch);
    }
}

#[test]
fn test_fail_outcome_smoke() {
    // Not a crash test; just ensures FailOutcome is wired through this binary.
    child_dispatch();
    let tmp = TempDir::new();
    let clock = ManualClock::new(1_000_000);
    let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
    let id = {
        use jobqueue_clean::EnqueueOpts;
        q.enqueue_opts(
            b"x",
            EnqueueOpts {
                max_attempts: Some(1),
                ..Default::default()
            },
        )
        .unwrap()
    };
    let lease = q.claim().unwrap().unwrap();
    let oc = q.fail(lease.id, lease.epoch).unwrap();
    assert_eq!(oc, FailOutcome::DeadLettered);
    assert!(q.is_dead(id));
}
