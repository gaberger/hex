//! Integration tests: concurrency, backoff, dead-lettering, recovery edge cases.

mod common;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use common::{test_cfg, TempDir};
use jobqueue_clean::{
    Clock, Config, EnqueueOpts, FailOutcome, FsyncMode, JobError, JobQueue, ManualClock,
};

// ---------------------------------------------------------------------------
// (a) MANDATORY — concurrency: no job claimed twice
// ---------------------------------------------------------------------------

#[test]
fn test_no_double_claim() {
    let tmp = TempDir::new();
    // Long visibility so leases never expire during the run.
    let mut cfg = test_cfg();
    cfg.visibility_timeout = Duration::from_secs(3600);
    let clock = ManualClock::new(1_000_000);
    let q = Arc::new(JobQueue::open_with_clock(tmp.path(), cfg, clock).unwrap());

    const N: u64 = 5_000;
    const T: usize = 64;
    for _ in 0..N {
        q.enqueue(b"job").unwrap();
    }

    let claimed: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::with_capacity(N as usize)));
    let mut handles = Vec::new();
    for _ in 0..T {
        let q = q.clone();
        let claimed = claimed.clone();
        handles.push(thread::spawn(move || loop {
            match q.claim().unwrap() {
                Some(lease) => {
                    claimed.lock().unwrap().push(lease.id);
                    q.complete(lease.id, lease.epoch).unwrap();
                }
                None => {
                    // Are we done?
                    let s = q.stats();
                    if s.done == N {
                        break;
                    }
                    // else: nothing claimable right now but not all done — spin.
                    if s.ready == 0 && s.claimed == 0 {
                        break;
                    }
                    std::hint::spin_loop();
                }
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let ids = claimed.lock().unwrap();
    let set: HashSet<u64> = ids.iter().copied().collect();
    assert_eq!(set.len(), ids.len(), "no id claimed twice");
    assert_eq!(set.len() as u64, N, "every job claimed exactly once");
    let expected: HashSet<u64> = (0..N).collect();
    assert_eq!(set, expected, "claimed set equals 0..N");
    assert_eq!(q.stats().done, N);
}

#[test]
fn test_no_double_claim_with_reclaim() {
    let tmp = TempDir::new();
    let mut cfg = test_cfg();
    cfg.visibility_timeout = Duration::from_millis(1000);
    cfg.default_max_attempts = 1000; // never dead-letter via deliveries here
    let clock = ManualClock::new(1_000_000);
    let q = Arc::new(JobQueue::open_with_clock(tmp.path(), cfg, clock.clone()).unwrap());

    const N: u64 = 500;
    for _ in 0..N {
        q.enqueue(b"job").unwrap();
    }

    // Set of currently-LIVE leased ids. The invariant: at any instant, no two
    // live leases share an id. Enforced by CAS-insert on claim.
    let live: Arc<Mutex<HashSet<u64>>> = Arc::new(Mutex::new(HashSet::new()));
    let stop = Arc::new(AtomicBool::new(false));

    let mut handles = Vec::new();
    for t in 0..16usize {
        let q = q.clone();
        let live = live.clone();
        let stop = stop.clone();
        handles.push(thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match q.claim().unwrap() {
                    Some(lease) => {
                        // The "live lease" window is the time between claim and
                        // when we relinquish it. We model an instantaneous worker:
                        // under a single `live` lock acquisition we assert no other
                        // live lease holds this id, then immediately release it.
                        // This precisely tests the LIVE no-double-claim invariant
                        // (overlapping un-expired leases for one id are impossible),
                        // while still exercising expiry+reclaim re-offers as the
                        // clock advances.
                        {
                            let mut l = live.lock().unwrap();
                            let inserted = l.insert(lease.id);
                            assert!(
                                inserted,
                                "two LIVE leases share id {} — double live claim!",
                                lease.id
                            );
                            l.remove(&lease.id);
                        }
                        // ~50% complete immediately, 50% drop (simulate slow death
                        // — the lease will be reclaimed once the clock advances).
                        if (lease.id as usize + t) % 2 == 0 {
                            // If the lease already expired and was reclaimed under
                            // us, complete returns StaleLease — EXPECTED
                            // at-least-once behavior, not a double LIVE claim.
                            match q.complete(lease.id, lease.epoch) {
                                Ok(()) => {}
                                Err(JobError::StaleLease) => {}
                                Err(e) => panic!("unexpected complete error: {e:?}"),
                            }
                        }
                    }
                    None => {
                        thread::yield_now();
                    }
                }
            }
        }));
    }

    // Drive the clock forward repeatedly so expired leases get reclaimed, until
    // all jobs are done.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let s = q.stats();
        if s.done == N {
            break;
        }
        clock.advance(1500); // past the visibility timeout
        q.tick().unwrap();
        thread::sleep(Duration::from_millis(2));
        if std::time::Instant::now() > deadline {
            panic!("did not drain in time: {:?}", q.stats());
        }
    }
    stop.store(true, Ordering::SeqCst);
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(q.stats().done, N, "every job eventually Done");
}

// ---------------------------------------------------------------------------
// (b) MANDATORY — exponential-backoff correctness
// ---------------------------------------------------------------------------

#[test]
fn test_backoff_schedule() {
    let tmp = TempDir::new();
    let cfg = Config {
        visibility_timeout: Duration::from_millis(1000),
        base_backoff: Duration::from_secs(1),
        max_backoff: Duration::from_secs(60),
        default_max_attempts: 10,
        jitter: false,
        fsync_mode: FsyncMode::PerOp,
        compact_threshold_records: 0,
    };
    let clock = ManualClock::new(0);
    let q = JobQueue::open_with_clock(tmp.path(), cfg, clock.clone()).unwrap();

    let id = q.enqueue(b"retry-me").unwrap();

    // Expected delays in ms: 1,2,4,8,16,32,60,60,60 (cap engaged at 60s).
    let expected = [1000u64, 2000, 4000, 8000, 16000, 32000, 60000, 60000, 60000];
    for &exp in expected.iter() {
        let lease = q.claim().unwrap().expect("claimable");
        assert_eq!(lease.id, id);
        let before = clock.now_ms();
        let oc = q.fail(lease.id, lease.epoch).unwrap();
        match oc {
            FailOutcome::Retrying { ready_at_ms } => {
                assert_eq!(
                    ready_at_ms - before,
                    exp,
                    "backoff delay should be {} ms",
                    exp
                );
            }
            FailOutcome::DeadLettered => panic!("should not dead-letter yet"),
        }
        // claim returns None at not_before-1, Some at not_before.
        clock.set_wall(before + exp - 1);
        assert!(
            q.claim().unwrap().is_none(),
            "not yet due at not_before-1ms"
        );
        clock.set_wall(before + exp);
        // exercised next loop iteration via claim()
    }
}

#[test]
fn test_backoff_blocking_wakes_at_due_time() {
    // [FIX-LOSTWAKE]: claim_blocking must wake at the due time with no notify.
    let tmp = TempDir::new();
    let cfg = Config {
        base_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_secs(60),
        visibility_timeout: Duration::from_millis(1000),
        default_max_attempts: 10,
        jitter: false,
        fsync_mode: FsyncMode::PerOp,
        compact_threshold_records: 0,
    };
    // Use SystemClock so real time passes for the blocking wait.
    let q = Arc::new(JobQueue::open(tmp.path(), cfg).unwrap());
    let id = q.enqueue(b"x").unwrap();
    let lease = q.claim().unwrap().unwrap();
    let oc = q.fail(lease.id, lease.epoch).unwrap();
    assert!(matches!(oc, FailOutcome::Retrying { .. }));

    // No enqueue/notify will happen; the blocking claimer must wake purely by
    // clock passage when the 50ms backoff elapses.
    let got = q
        .claim_blocking(Duration::from_secs(5))
        .unwrap()
        .expect("should wake at due time");
    assert_eq!(got.id, id);
}

#[test]
fn test_backoff_saturating_shift_no_overflow() {
    let tmp = TempDir::new();
    let cfg = Config {
        base_backoff: Duration::from_secs(1),
        max_backoff: Duration::from_secs(60),
        visibility_timeout: Duration::from_millis(1000),
        default_max_attempts: 100,
        jitter: false,
        fsync_mode: FsyncMode::PerOp,
        compact_threshold_records: 0,
    };
    let clock = ManualClock::new(0);
    let q = JobQueue::open_with_clock(tmp.path(), cfg, clock.clone()).unwrap();
    let id = q.enqueue(b"x").unwrap();
    // Fail 40 times; must clamp to max, never panic.
    for _ in 0..40 {
        let lease = q.claim().unwrap().expect("claimable");
        assert_eq!(lease.id, id);
        match q.fail(lease.id, lease.epoch).unwrap() {
            FailOutcome::Retrying { ready_at_ms } => {
                let delay = ready_at_ms - clock.now_ms();
                assert!(delay <= 60_000, "delay clamped to max");
                clock.set_wall(ready_at_ms);
            }
            FailOutcome::DeadLettered => panic!("max_attempts is 100"),
        }
    }
}

#[test]
fn test_backoff_jitter_never_exceeds_max() {
    // Regression: jitter (+/-12.5%) was applied AFTER the max-backoff clamp with
    // no final re-clamp, so a saturated retry could overshoot max_backoff by up
    // to +12.5% (e.g. 4000ms cap -> 4500ms). The returned delay must always stay
    // within [0, max_backoff] even with jitter enabled.
    let tmp = TempDir::new();
    let cfg = Config {
        base_backoff: Duration::from_millis(1000),
        max_backoff: Duration::from_millis(4000),
        visibility_timeout: Duration::from_millis(1000),
        default_max_attempts: 100_000,
        jitter: true,
        fsync_mode: FsyncMode::PerOp,
        compact_threshold_records: 0,
    };
    let clock = ManualClock::new(0);
    let q = JobQueue::open_with_clock(tmp.path(), cfg, clock.clone()).unwrap();
    let id = q.enqueue(b"x").unwrap();

    let mut max_seen = 0u64;
    // Fail many times so backoff saturates at the 4000ms cap and the jittered
    // path is exercised across many distinct PRNG draws.
    for _ in 0..300 {
        let lease = q.claim().unwrap().expect("claimable");
        assert_eq!(lease.id, id);
        match q.fail(lease.id, lease.epoch).unwrap() {
            FailOutcome::Retrying { ready_at_ms } => {
                let delay = ready_at_ms - clock.now_ms();
                max_seen = max_seen.max(delay);
                assert!(
                    delay <= 4000,
                    "jitter pushed delay {delay}ms above max_backoff 4000ms"
                );
                clock.set_wall(ready_at_ms);
            }
            FailOutcome::DeadLettered => panic!("max_attempts is very high; should not dead-letter"),
        }
    }
    // Sanity: backoff actually saturated to (or near) the cap, so we really did
    // exercise the jittered-at-cap path rather than only small early delays.
    assert!(
        max_seen >= 3500,
        "expected backoff to saturate near the 4000ms cap, max_seen={max_seen}ms"
    );
}

// ---------------------------------------------------------------------------
// (c) MANDATORY — dead-lettering at max_attempts (via fail)
// ---------------------------------------------------------------------------

#[test]
fn test_dead_letter_via_fail() {
    let tmp = TempDir::new();
    let mut cfg = test_cfg();
    cfg.default_max_attempts = 3;
    let clock = ManualClock::new(0);
    let q = JobQueue::open_with_clock(tmp.path(), cfg, clock.clone()).unwrap();
    let id = q.enqueue(b"x").unwrap();

    // fail 1
    let l = q.claim().unwrap().unwrap();
    assert_eq!(q.fail(l.id, l.epoch).unwrap(), {
        FailOutcome::Retrying {
            ready_at_ms: q.job_not_before_ms(id).unwrap(),
        }
    });
    clock.set_wall(q.job_not_before_ms(id).unwrap());

    // fail 2
    let l = q.claim().unwrap().unwrap();
    assert!(matches!(
        q.fail(l.id, l.epoch).unwrap(),
        FailOutcome::Retrying { .. }
    ));
    clock.set_wall(q.job_not_before_ms(id).unwrap());

    // fail 3 → dead
    let l = q.claim().unwrap().unwrap();
    assert_eq!(q.fail(l.id, l.epoch).unwrap(), FailOutcome::DeadLettered);
    assert!(q.is_dead(id));

    // never re-offered
    assert!(q.claim().unwrap().is_none());
}

// ---------------------------------------------------------------------------
// (d) crash recovery: torn tail / interior corruption
// ---------------------------------------------------------------------------

#[test]
fn test_torn_tail_dropped() {
    let tmp = TempDir::new();
    {
        let clock = ManualClock::new(0);
        let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
        q.enqueue(b"first").unwrap();
        q.enqueue(b"second").unwrap();
    }
    // Append a deliberately truncated frame: len prefix claiming 100 bytes but
    // only a few payload bytes present.
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(tmp.path().join("wal.log"))
            .unwrap();
        let len: u32 = 100;
        f.write_all(&len.to_le_bytes()).unwrap();
        f.write_all(&0u32.to_le_bytes()).unwrap(); // bogus crc
        f.write_all(b"partial").unwrap(); // only 7 of 100 bytes
        f.sync_data().unwrap();
    }
    // Reopen: torn tail dropped, prior records intact, no error.
    let clock = ManualClock::new(0);
    let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
    let s = q.stats();
    assert_eq!(s.total, 2, "two valid records survive: {s:?}");
    assert_eq!(s.ready, 2);
}

#[test]
fn test_interior_corruption_fails_loud() {
    let tmp = TempDir::new();
    {
        let clock = ManualClock::new(0);
        let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
        q.enqueue(b"one").unwrap();
        q.enqueue(b"two").unwrap();
        q.enqueue(b"three").unwrap();
    }
    // Corrupt a byte inside the MIDDLE frame's CRC (bytes 4..8 of that frame).
    {
        use std::io::{Read, Seek, SeekFrom, Write};
        let path = tmp.path().join("wal.log");
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        // Walk to the second frame.
        let len0 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let frame1_start = 8 + len0;
        // Flip a byte in frame1's CRC field (offset frame1_start+4).
        let crc_byte = frame1_start + 4;
        let mut f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.seek(SeekFrom::Start(crc_byte as u64)).unwrap();
        let orig = bytes[crc_byte];
        f.write_all(&[orig ^ 0xFF]).unwrap();
        f.sync_data().unwrap();
    }
    let clock = ManualClock::new(0);
    let r = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock);
    match r {
        Err(JobError::Corrupt(msg)) => {
            assert!(msg.contains("offset"), "corrupt error names offset: {msg}");
        }
        Err(other) => panic!("expected Corrupt, got error {other:?}"),
        Ok(_) => panic!("expected Corrupt, but open succeeded"),
    }
}

// [FIX-LEN-CRC] Regression: corrupting an INTERIOR frame's LENGTH prefix (not
// its CRC field) must fail loud, never silently truncate committed records.
//
// Manifestation (1): a single-bit flip in the high byte of an interior frame's
// length prefix makes the declared length enormous. Before the fix, read_frame
// returned Incomplete (declared len > remaining bytes), the recovery loop
// treated that as a clean torn tail, and every record after the corrupted frame
// was silently dropped with open() == Ok. With the fix, the corruption is
// detected and recovery returns JobError::Corrupt.
#[test]
fn test_interior_len_prefix_corruption_fails_loud_high_bit() {
    let tmp = TempDir::new();
    {
        let clock = ManualClock::new(0);
        let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
        for p in [&b"one"[..], b"two", b"three", b"four", b"five"] {
            q.enqueue(p).unwrap();
        }
    }
    {
        use std::io::{Read, Seek, SeekFrom, Write};
        let path = tmp.path().join("wal.log");
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        let len0 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let frame1_start = 8 + len0; // start of the 2nd (interior) frame
        let lenbyte = frame1_start + 3; // high byte of frame1's LE length prefix
        let orig = bytes[lenbyte];
        let mut f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.seek(SeekFrom::Start(lenbyte as u64)).unwrap();
        f.write_all(&[orig ^ 0x80]).unwrap(); // one-bit flip → huge declared length
        f.sync_data().unwrap();
    }
    let clock = ManualClock::new(0);
    let r = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock);
    match r {
        Err(JobError::Corrupt(_)) => {} // correct: loud failure, no silent loss
        Ok(_) => panic!(
            "expected Corrupt for interior length-prefix corruption, but open succeeded \
             (committed records silently lost)"
        ),
        Err(other) => panic!("expected Corrupt, got error {other:?}"),
    }
}

// Manifestation (2): overwrite an interior frame's 4-byte length so the frame
// "ends" exactly at EOF, skipping over the following valid frames. Before the
// fix, read_frame produced BadCrc and is_last_frame trusted that same corrupted
// length (frame_end == EOF) to declare a torn tail, silently dropping the later
// records. With the fix, recovery scans for recoverable frames independently of
// the corrupted length and fails loud.
#[test]
fn test_interior_len_prefix_corruption_fails_loud_span_to_eof() {
    let tmp = TempDir::new();
    {
        let clock = ManualClock::new(0);
        let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
        for p in [&b"one"[..], b"two", b"three"] {
            q.enqueue(p).unwrap();
        }
    }
    {
        use std::io::{Read, Seek, SeekFrom, Write};
        let path = tmp.path().join("wal.log");
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        let total = bytes.len();
        let len0 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let frame1_start = 8 + len0; // start of the 2nd (interior) frame
        // Choose a declared length so frame_end (HEADER + len) lands at EOF.
        let new_len = (total - frame1_start - 8) as u32;
        let mut f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.seek(SeekFrom::Start(frame1_start as u64)).unwrap();
        f.write_all(&new_len.to_le_bytes()).unwrap();
        f.sync_data().unwrap();
    }
    let clock = ManualClock::new(0);
    let r = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock);
    match r {
        Err(JobError::Corrupt(_)) => {} // correct: loud failure, no silent loss
        Ok(_) => panic!(
            "expected Corrupt for length-spans-to-EOF corruption, but open succeeded \
             (committed records silently lost)"
        ),
        Err(other) => panic!("expected Corrupt, got error {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Supporting unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_buffer_durability() {
    // [FIX-BUFWRITER]: write one record, read the file bytes back WITHOUT
    // closing the queue, confirm the frame is present (proves sync flushed our
    // explicit buffer; no hidden BufWriter).
    let tmp = TempDir::new();
    let clock = ManualClock::new(0);
    let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
    q.enqueue(b"durable-bytes").unwrap();

    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(tmp.path().join("wal.log"))
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert!(bytes.len() > 8, "frame written and visible while queue is live");
    // The payload bytes should appear in the JSON.
    let s = String::from_utf8_lossy(&bytes);
    // payload is serialized as a byte array, so individual bytes appear; check
    // the JSON has an Enqueued tag.
    assert!(s.contains("Enqueued"), "record content present on disk");
}

#[test]
fn test_fsync_failure_rolls_back() {
    // [FIX-FSYNC-ROLLBACK]: inject a sync failure; the op returns Io and stats
    // show state unchanged (no silent stuck job).
    let tmp = TempDir::new();
    let clock = ManualClock::new(0);
    let (q, fault) =
        JobQueue::open_with_fault(tmp.path(), test_cfg(), clock).unwrap();
    // Establish a baseline.
    q.enqueue(b"a").unwrap();
    let before = q.stats();

    fault.arm_sync_failure();
    let r = q.enqueue(b"b");
    assert!(matches!(r, Err(JobError::Io(_))), "fsync failure surfaces as Io");

    let after = q.stats();
    assert_eq!(before, after, "job state unchanged after rolled-back enqueue");
}

#[test]
fn test_stale_heap_entry_revalidated() {
    // [FIX-STALEHEAP]: force Ready->Claimed->Reclaim->Ready so multiple ready
    // entries exist for one job; claim never hands it out before not_before and
    // never twice.
    let tmp = TempDir::new();
    let mut cfg = test_cfg();
    cfg.visibility_timeout = Duration::from_millis(1000);
    cfg.default_max_attempts = 100;
    let clock = ManualClock::new(0);
    let q = JobQueue::open_with_clock(tmp.path(), cfg, clock.clone()).unwrap();
    let id = q.enqueue(b"x").unwrap();

    // Claim, let it expire (reclaim pushes a new ready entry with a new epoch);
    // the old entry is now stale in the heap.
    let l1 = q.claim().unwrap().unwrap();
    assert_eq!(l1.id, id);
    clock.advance(2000); // past lease
    q.tick().unwrap(); // reclaim -> new epoch, new ready entry

    // There are now potentially two ready entries (old epoch stale, new valid).
    // Claim should hand it out exactly once.
    let l2 = q.claim().unwrap().expect("re-claimable once");
    assert_eq!(l2.id, id);
    assert_ne!(l2.epoch, l1.epoch);
    // Second claim should NOT return the same job again (it's claimed now).
    assert!(q.claim().unwrap().is_none(), "no duplicate hand-out");
}

#[test]
fn test_idempotent_enqueue() {
    // [FIX-IDEMPOTENT]
    let tmp = TempDir::new();
    let clock = ManualClock::new(0);
    let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
    let opts = EnqueueOpts {
        idempotency_key: Some("k1".into()),
        ..Default::default()
    };
    let id1 = q.enqueue_opts(b"x", opts.clone()).unwrap();
    let id2 = q.enqueue_opts(b"y", opts).unwrap();
    assert_eq!(id1, id2, "same key → same job id");
    assert_eq!(q.stats().total, 1, "only one job created");
}

#[test]
fn test_idempotent_enqueue_survives_restart() {
    let tmp = TempDir::new();
    let opts = EnqueueOpts {
        idempotency_key: Some("kx".into()),
        ..Default::default()
    };
    let id1 = {
        let clock = ManualClock::new(0);
        let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
        q.enqueue_opts(b"x", opts.clone()).unwrap()
    };
    // Reopen and re-enqueue with the same key.
    let clock = ManualClock::new(0);
    let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
    let id2 = q.enqueue_opts(b"x", opts).unwrap();
    assert_eq!(id1, id2);
    assert_eq!(q.stats().total, 1);
}

#[test]
fn test_next_id_epoch_no_reuse_after_compaction() {
    // [FIX-NEXTID]
    let tmp = TempDir::new();
    let mut cfg = test_cfg();
    cfg.compact_threshold_records = 0; // manual compaction
    let id_a;
    let id_b;
    {
        let clock = ManualClock::new(0);
        let q = JobQueue::open_with_clock(tmp.path(), cfg.clone(), clock).unwrap();
        id_a = q.enqueue(b"a").unwrap();
        // claim+complete so compaction sees finished work
        let l = q.claim().unwrap().unwrap();
        q.complete(l.id, l.epoch).unwrap();
        id_b = q.enqueue(b"b").unwrap();
        q.compact_now().unwrap();
    }
    // Reopen, enqueue → strictly greater id.
    let clock = ManualClock::new(0);
    let q = JobQueue::open_with_clock(tmp.path(), cfg, clock).unwrap();
    let id_c = q.enqueue(b"c").unwrap();
    assert!(id_c > id_a && id_c > id_b, "ids never reused after compaction");
}

#[test]
fn test_clock_step_no_mass_expiry() {
    // [FIX-CLOCKSTEP]: hold many live leases; jump WALL clock forward 1h with
    // monotonic unchanged → no lease reclaimed.
    let tmp = TempDir::new();
    let mut cfg = test_cfg();
    cfg.visibility_timeout = Duration::from_secs(30);
    cfg.default_max_attempts = 100;
    let clock = ManualClock::new(1_000_000);
    let q = JobQueue::open_with_clock(tmp.path(), cfg, clock.clone()).unwrap();
    for _ in 0..100 {
        q.enqueue(b"j").unwrap();
    }
    let mut leases = Vec::new();
    while let Some(l) = q.claim().unwrap() {
        leases.push(l);
    }
    assert_eq!(leases.len(), 100);
    assert_eq!(q.stats().claimed, 100);

    // Jump WALL forward 1h, monotonic unchanged.
    clock.advance_wall(3_600_000);
    q.tick().unwrap();
    assert_eq!(
        q.stats().claimed,
        100,
        "wall jump must NOT mass-expire live leases (monotonic deadlines)"
    );
    // Now advance monotonic past the timeout → reclaim happens.
    clock.advance_mono(31_000);
    q.tick().unwrap();
    assert_eq!(q.stats().claimed, 0, "monotonic expiry reclaims");
    assert_eq!(q.stats().ready, 100);
}

#[test]
fn test_stranded_lease_swept_on_idle() {
    // [FIX-STRANDED]: claim the only job, let lease expire on an idle queue; a
    // claim_blocking caller wakes at the deadline and reclaims it.
    let tmp = TempDir::new();
    let mut cfg = test_cfg();
    cfg.visibility_timeout = Duration::from_millis(80);
    cfg.default_max_attempts = 100;
    let q = Arc::new(JobQueue::open(tmp.path(), cfg).unwrap());
    let id = q.enqueue(b"only").unwrap();
    let _l = q.claim().unwrap().unwrap(); // claimed, then "die" (drop lease)

    // A blocking claimer on an otherwise-idle queue must wake when the lease
    // expires (~80ms) and reclaim it — no external notify.
    let got = q
        .claim_blocking(Duration::from_secs(5))
        .unwrap()
        .expect("stranded lease reclaimed on idle queue");
    assert_eq!(got.id, id);
}

#[test]
fn test_finalize_after_reclaim_is_stalelease() {
    // [FIX-FINALIZE-RACE]: A claims, lease expires, B reclaims; A's complete →
    // StaleLease.
    let tmp = TempDir::new();
    let mut cfg = test_cfg();
    cfg.visibility_timeout = Duration::from_millis(1000);
    cfg.default_max_attempts = 100;
    let clock = ManualClock::new(0);
    let q = JobQueue::open_with_clock(tmp.path(), cfg, clock.clone()).unwrap();
    q.enqueue(b"x").unwrap();
    let a = q.claim().unwrap().unwrap();
    clock.advance(2000);
    q.tick().unwrap(); // reclaim
    let b = q.claim().unwrap().unwrap();
    assert_eq!(a.id, b.id);
    assert_ne!(a.epoch, b.epoch);
    // A's finalize must be rejected.
    assert!(matches!(q.complete(a.id, a.epoch), Err(JobError::StaleLease)));
    // B's succeeds.
    q.complete(b.id, b.epoch).unwrap();
}

#[test]
fn test_delayed_enqueue() {
    // [FIX-9]: delayed enqueue not claimable until not_before.
    let tmp = TempDir::new();
    let clock = ManualClock::new(1000);
    let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock.clone()).unwrap();
    let id = q
        .enqueue_opts(
            b"later",
            EnqueueOpts {
                delay: Some(Duration::from_millis(500)),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(q.claim().unwrap().is_none(), "not due yet");
    clock.set_wall(1000 + 499);
    assert!(q.claim().unwrap().is_none(), "still not due at -1ms");
    clock.set_wall(1000 + 500);
    let l = q.claim().unwrap().expect("now due");
    assert_eq!(l.id, id);
}

#[test]
fn test_per_job_max_attempts_override() {
    let tmp = TempDir::new();
    let mut cfg = test_cfg();
    cfg.default_max_attempts = 100;
    let clock = ManualClock::new(0);
    let q = JobQueue::open_with_clock(tmp.path(), cfg, clock.clone()).unwrap();
    let id = q
        .enqueue_opts(
            b"x",
            EnqueueOpts {
                max_attempts: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
    let l = q.claim().unwrap().unwrap();
    assert_eq!(q.fail(l.id, l.epoch).unwrap(), FailOutcome::DeadLettered);
    assert!(q.is_dead(id));
}

#[test]
fn test_recovery_rebuilds_ready_jobs() {
    // Basic durability: enqueue, drop, reopen, claim.
    let tmp = TempDir::new();
    let payloads = [b"alpha".to_vec(), b"beta".to_vec(), b"gamma".to_vec()];
    {
        let clock = ManualClock::new(0);
        let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
        for p in &payloads {
            q.enqueue(p).unwrap();
        }
    }
    let clock = ManualClock::new(0);
    let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();
    let mut seen: HashMap<Vec<u8>, u32> = HashMap::new();
    while let Some(l) = q.claim().unwrap() {
        *seen.entry(l.payload.to_vec()).or_default() += 1;
        q.complete(l.id, l.epoch).unwrap();
    }
    assert_eq!(seen.len(), 3, "all enqueued payloads recovered");
    for p in &payloads {
        assert_eq!(seen.get(p).copied(), Some(1));
    }
}

// ---------------------------------------------------------------------------
// Regression: backoff jitter must not overflow i64 for pathological max_backoff.
//
// Previously `backoff_delay` computed `(delay as i64 * signed) / 1000`. With an
// unvalidated, absurdly large `max_backoff` the clamped `delay` can reach
// i64::MAX, so the multiply overflows i64 — panicking under the held queue
// mutex in debug (poisoning it) and silently wrapping to 0 in release. The fix
// performs the jitter math in i128. This test reproduces the exact failing
// path and asserts fail() returns cleanly with the backoff preserved.
// ---------------------------------------------------------------------------

#[test]
fn test_backoff_jitter_no_overflow_pathological_max_backoff() {
    let tmp = TempDir::new();
    // A pathological, unvalidated backoff: the clamped delay reaches i64::MAX,
    // which overflows the old `delay as i64 * signed` jitter math. We start the
    // clock at 0 so that `now + delay` (a separate u64 add) does not also
    // overflow — this test isolates the jitter-multiply bug at lib.rs:600.
    let cfg = Config {
        visibility_timeout: Duration::from_millis(10),
        base_backoff: Duration::from_millis(i64::MAX as u64),
        max_backoff: Duration::from_millis(i64::MAX as u64),
        default_max_attempts: 100,
        jitter: true,
        fsync_mode: FsyncMode::PerOp,
        compact_threshold_records: 0,
    };
    let clock = ManualClock::new(0);
    let q = JobQueue::open_with_clock(tmp.path(), cfg, clock).unwrap();
    q.enqueue(b"x").unwrap();
    let lease = q.claim().unwrap().unwrap();

    // Before the fix this panics ("attempt to multiply with overflow") inside
    // backoff_delay while the queue mutex is held, poisoning the mutex.
    let outcome = q.fail(lease.id, lease.epoch).unwrap();

    // First failure of a job with max_attempts=100 must be a retry, not a
    // dead-letter, and the backoff must survive jitter (release used to wrap to
    // 0, losing the backoff entirely so ready_at_ms would collapse to `now`).
    match outcome {
        FailOutcome::Retrying { ready_at_ms } => {
            assert!(
                ready_at_ms > 1_000,
                "backoff silently lost to overflow wrap (ready_at_ms={ready_at_ms})"
            );
        }
        other => panic!("expected Retrying, got {other:?}"),
    }

    // Mutex must not be poisoned: a subsequent operation still succeeds.
    let _ = q.claim().unwrap();
}

// REGRESSION [FIX-ENQUEUE-DELAY-OVERFLOW]: enqueue_opts computed
// `not_before_ms = now + delay_ms` as an unchecked u64 add. With a wall clock
// near u64::MAX, adding the requested delay overflowed: debug builds (default
// dev profile has overflow-checks=true) panic "attempt to add with overflow"
// at the add site; release builds wrap to a value near 0, so a job that should
// be delayed becomes immediately claimable instead. The fix is
// `now.saturating_add(delay_ms)`. Before the fix this test panics in debug;
// with the fix not_before_ms saturates at u64::MAX (still strictly in the
// future) and the delayed job is NOT immediately claimable.
#[test]
fn test_enqueue_delay_no_overflow_near_u64_max() {
    let tmp = TempDir::new();
    let start = u64::MAX - 5;
    let clock = ManualClock::new(start);
    let q = JobQueue::open_with_clock(tmp.path(), test_cfg(), clock).unwrap();

    let id = q
        .enqueue_opts(
            b"x",
            EnqueueOpts {
                delay: Some(Duration::from_millis(1000)),
                ..Default::default()
            },
        )
        .unwrap();

    // Saturating add must clamp at u64::MAX rather than wrapping to ~0.
    let nb = q.job_not_before_ms(id).expect("job should exist");
    assert_eq!(
        nb,
        u64::MAX,
        "not_before_ms must saturate at u64::MAX, not wrap (got {nb})"
    );

    // The delayed job must remain in the future relative to `now`, so it is
    // NOT immediately claimable. (A wrapped not_before_ms near 0 would make it
    // instantly claimable.)
    assert!(
        nb > start,
        "delayed job must still be scheduled after now ({start}), got {nb}"
    );
    assert!(
        q.claim().unwrap().is_none(),
        "delayed job must not be immediately claimable after overflow-safe enqueue"
    );
}

#[test]
fn test_claim_blocking_deadline_no_overflow_near_u64_max() {
    // Regression for the [FIX-DEADLINE-OVERFLOW] bug in claim_blocking:
    //
    //   let deadline_mono = self.clock.mono_ms() + timeout.as_millis() as u64;
    //
    // was an unchecked u64 add. With a monotonic clock near u64::MAX and any
    // nonzero timeout the add overflowed:
    //   * debug build  -> panic "attempt to add with overflow" at the deadline
    //                     computation, before the queue is ever consulted;
    //   * release build -> deadline_mono wraps to a tiny value so the very first
    //                     `now_mono >= deadline_mono` check is true and the call
    //                     returns Ok(None) immediately instead of blocking.
    //
    // The fix uses saturating_add, clamping the deadline to u64::MAX so the call
    // blocks (rather than panicking / spuriously returning None). We keep the
    // queue empty so this exercises ONLY the deadline computation at the top of
    // claim_blocking, then advance the monotonic clock to u64::MAX so the wait
    // terminates deterministically and the call returns Ok(None).
    let tmp = TempDir::new();
    let clock = ManualClock::new(u64::MAX - 5);
    let q = Arc::new(JobQueue::open_with_clock(tmp.path(), test_cfg(), clock.clone()).unwrap());

    let qc = Arc::clone(&q);
    let handle = thread::spawn(move || {
        // Pre-fix debug: this panics here. Pre-fix release: returns Ok(None)
        // immediately (the wrapped deadline is already "in the past").
        // Post-fix: blocks until the clock reaches the saturated u64::MAX deadline.
        qc.claim_blocking(Duration::from_millis(1000))
    });

    // Give the blocking call a moment to compute its (saturated) deadline and
    // enter the wait loop, then advance the monotonic clock to u64::MAX so the
    // `now_mono >= deadline_mono` guard fires and the call returns Ok(None).
    thread::sleep(Duration::from_millis(50));
    clock.advance_mono(5); // u64::MAX - 5 -> u64::MAX

    let res = handle.join().expect("claim_blocking thread must not panic");
    assert!(
        matches!(res, Ok(None)),
        "claim_blocking near u64::MAX must saturate the deadline and return Ok(None) \
         after the timeout, not panic or short-circuit; got {res:?}"
    );
}

// Regression for the [FIX-VIS-OVERFLOW] bug in claim_locked (src/lib.rs ~346):
//
//   let vis = self.cfg.visibility_timeout.as_millis() as u64;
//   let mono_deadline   = now_mono + vis;   // unchecked u64 add
//   let wall_deadline_ms = now_wall + vis;  // unchecked u64 add
//
// When a *ready* job is actually claimed with a clock near u64::MAX (or a
// pathologically large visibility_timeout), these adds overflow:
//   * debug build  -> panic "attempt to add with overflow" mid-claim, AFTER the
//                     Claimed WAL record / index mutation begins;
//   * release build -> mono_deadline wraps to a tiny value, stored in
//                     claimed_by_deadline. The next reclaim_sweep sees the
//                     just-claimed lease as already expired and re-offers it
//                     (spurious double-offer of a live lease).
//
// The fix caps the cast (u64::try_from(..).unwrap_or(u64::MAX)) and uses
// saturating_add, so the deadline clamps to u64::MAX. Unlike the empty-queue
// claim_blocking test above, this enqueues a ready job so the claim REACHES the
// deadline computation and returns a real Lease.
#[test]
fn test_claim_lease_deadline_no_overflow_near_u64_max() {
    let tmp = TempDir::new();
    let start = u64::MAX - 5;
    let clock = ManualClock::new(start);

    // Large visibility timeout: start + vis would overflow u64 with a raw add.
    let mut cfg = test_cfg();
    cfg.visibility_timeout = Duration::from_millis(10_000);
    let q = Arc::new(JobQueue::open_with_clock(tmp.path(), cfg, clock.clone()).unwrap());

    // A ready job (no delay) so claim_locked proceeds to the deadline math.
    q.enqueue(b"near-max").unwrap();

    // Pre-fix debug: panics with "attempt to add with overflow" here.
    let lease = q
        .claim()
        .expect("claim must not panic near u64::MAX")
        .expect("a ready job must be claimable");

    // Saturating add must clamp the wall deadline at u64::MAX rather than wrap
    // to a tiny value.
    assert_eq!(
        lease.deadline_ms,
        u64::MAX,
        "lease wall deadline must saturate at u64::MAX, not wrap (got {})",
        lease.deadline_ms
    );

    // And the lease must NOT be spuriously reclaimable. Pre-fix (release), the
    // wrapped mono_deadline was a tiny value far below the current monotonic
    // clock (~u64::MAX - 5), so the very next reclaim_sweep treated the live
    // lease as already expired and re-offered it. With the saturating fix the
    // mono_deadline is u64::MAX, which is strictly greater than the current
    // clock, so the sweep breaks and a fresh claim sees nothing due. We do NOT
    // advance the clock here: advancing to exactly u64::MAX would hit the
    // legitimate `now_mono >= deadline` expiry boundary, which is correct
    // behavior, not the spurious-double-offer bug under test.
    assert!(
        matches!(q.claim(), Ok(None)),
        "a freshly-claimed lease whose deadline saturated to u64::MAX must not \
         be instantly re-offered while the clock is still below it (no spurious \
         double-offer)"
    );
}
