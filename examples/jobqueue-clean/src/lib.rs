//! # jobqueue-clean — a concurrent, durable, crash-safe job queue
//!
//! ## Delivery semantics: AT-LEAST-ONCE (read this).
//!
//! This queue provides **at-least-once** delivery with a visibility-timeout
//! lease. This is a documented, non-negotiable property — not a defect.
//! Exactly-once delivery is impossible across a crash with side effects.
//!
//! What we DO guarantee:
//!  * No two LIVE workers ever both successfully `complete()` (or `fail()`) the
//!    same job — finalization is fenced by a monotonic `epoch` token.
//!  * Every job that ends in-flight at a crash is re-offered exactly once per
//!    recovery, while still respecting `max_attempts` and dead-lettering.
//!  * Every value a caller observes (a `JobId`, a `Lease`, a `FailOutcome`) is
//!    on durable storage before the call returns.
//!
//! What we do NOT guarantee:
//!  * That a job is *processed* at most once. A slow-but-alive worker whose
//!    lease expired can still be running its handler concurrently with the
//!    re-claimer. The mutex serializes index mutation, NOT side effects.
//!
//! ## Therefore: HANDLERS MUST BE IDEMPOTENT.
//! The recommended dedup key for handler side effects is `(lease.id, lease.epoch)`.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

mod clock;
mod crc;
mod durable;
mod recovery;
mod wal;

pub use clock::{Clock, ManualClock, SystemClock};
pub use durable::FaultInjector;

use durable::Durable;
use wal::{FailedOutcomeRec, WalRecord};

pub type JobId = u64;

#[derive(Clone, Debug)]
pub struct Config {
    pub visibility_timeout: Duration,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
    pub default_max_attempts: u32,
    pub jitter: bool,
    pub fsync_mode: FsyncMode,
    pub compact_threshold_records: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            visibility_timeout: Duration::from_secs(30),
            base_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(300),
            default_max_attempts: 5,
            jitter: true,
            fsync_mode: FsyncMode::PerOp,
            compact_threshold_records: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum FsyncMode {
    PerOp,
    GroupCommit { window: Duration },
}

#[derive(Clone, Debug, Default)]
pub struct EnqueueOpts {
    pub max_attempts: Option<u32>,
    pub delay: Option<Duration>,
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Lease {
    pub id: JobId,
    pub epoch: u64,
    pub payload: Arc<[u8]>,
    pub attempts: u32,
    pub deadline_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailOutcome {
    Retrying { ready_at_ms: u64 },
    DeadLettered,
}

#[derive(Debug)]
pub enum JobError {
    NotFound,
    /// The job is not currently leased under this (id, epoch) — you lost the
    /// race; someone else owns it now. Do NOT retry the finalize.
    StaleLease,
    Io(io::Error),
    Corrupt(String),
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobError::NotFound => write!(f, "job not found"),
            JobError::StaleLease => write!(f, "stale lease (someone else owns this job)"),
            JobError::Io(e) => write!(f, "io error: {e}"),
            JobError::Corrupt(s) => write!(f, "corrupt wal: {s}"),
        }
    }
}

impl std::error::Error for JobError {}

impl From<io::Error> for JobError {
    fn from(e: io::Error) -> Self {
        JobError::Io(e)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub ready: u64,
    pub claimed: u64,
    pub done: u64,
    pub dead: u64,
    pub total: u64,
}

// ---------------------------------------------------------------------------
// Internal data model
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub(crate) enum Status {
    Ready {
        not_before_ms: u64,
    },
    Claimed {
        // Both deadlines are kept for documentation/inspection; live reclaim is
        // driven by the `claimed_by_deadline` heap rather than these fields.
        #[allow(dead_code)]
        mono_deadline: u64,
        #[allow(dead_code)]
        wall_deadline_ms: u64,
    },
    Done,
    Dead,
}

#[derive(Clone, Debug)]
pub(crate) struct Job {
    pub id: JobId,
    pub payload: Arc<[u8]>,
    pub attempts: u32,
    pub max_attempts: u32,
    pub lease_epoch: u64,
    pub deliveries: u32,
    pub status: Status,
    pub not_before_ms: u64,
}

pub(crate) struct Inner {
    pub jobs: HashMap<JobId, Job>,
    /// LAZY-DELETION heap of (not_before_ms, id, epoch). [FIX-STALEHEAP]
    pub ready_by_time: BinaryHeap<Reverse<(u64, JobId, u64)>>,
    /// LAZY-DELETION heap of (mono_deadline, id, epoch). [FIX-STALEHEAP]
    pub claimed_by_deadline: BinaryHeap<Reverse<(u64, JobId, u64)>>,
    pub next_id: u64,
    pub next_epoch: u64,
    pub idem: HashMap<String, JobId>,
    pub durable: Durable,
    /// Counter feeding deterministic jitter without external rng.
    pub jitter_ctr: u64,
}

pub struct JobQueue {
    inner: Mutex<Inner>,
    not_empty: Condvar,
    cfg: Config,
    clock: Arc<dyn Clock>,
    #[allow(dead_code)]
    dir: PathBuf,
    fault: Arc<FaultInjector>,
}

impl JobQueue {
    pub fn open(dir: impl AsRef<Path>, cfg: Config) -> Result<Self, JobError> {
        Self::open_with_clock(dir, cfg, Arc::new(SystemClock::new()))
    }

    pub fn open_with_clock(
        dir: impl AsRef<Path>,
        cfg: Config,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, JobError> {
        Self::open_full(dir, cfg, clock, Arc::new(FaultInjector::default()))
    }

    /// Test entry point that also returns the fault injector handle.
    pub fn open_with_fault(
        dir: impl AsRef<Path>,
        cfg: Config,
        clock: Arc<dyn Clock>,
    ) -> Result<(Self, Arc<FaultInjector>), JobError> {
        let fault = Arc::new(FaultInjector::default());
        let q = Self::open_full(dir, cfg, clock, fault.clone())?;
        Ok((q, fault))
    }

    fn open_full(
        dir: impl AsRef<Path>,
        cfg: Config,
        clock: Arc<dyn Clock>,
        fault: Arc<FaultInjector>,
    ) -> Result<Self, JobError> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;

        let inner = recovery::recover(&dir, &cfg, clock.as_ref(), fault.clone())?;

        Ok(Self {
            inner: Mutex::new(inner),
            not_empty: Condvar::new(),
            cfg,
            clock,
            dir,
            fault,
        })
    }

    /// Access the fault injector (tests).
    pub fn fault(&self) -> Arc<FaultInjector> {
        self.fault.clone()
    }

    fn lock(&self) -> Result<MutexGuard<'_, Inner>, JobError> {
        // We never panic under the lock, so poisoning only happens on an
        // external abort. Surface as Io so the caller restarts → log replay.
        // [FIX-POISON][FIX-POISON-DIVERGE]
        self.inner
            .lock()
            .map_err(|_| JobError::Io(io::Error::new(io::ErrorKind::Other, "lock poisoned")))
    }

    pub fn enqueue(&self, payload: &[u8]) -> Result<JobId, JobError> {
        self.enqueue_opts(payload, EnqueueOpts::default())
    }

    pub fn enqueue_opts(&self, payload: &[u8], opts: EnqueueOpts) -> Result<JobId, JobError> {
        let now = self.clock.now_ms();
        let mut g = self.lock()?;

        // Idempotency dedup. [FIX-IDEMPOTENT]
        if let Some(key) = &opts.idempotency_key {
            if let Some(&existing) = g.idem.get(key) {
                return Ok(existing);
            }
        }

        let id = g.next_id;
        let max_attempts = opts.max_attempts.unwrap_or(self.cfg.default_max_attempts);
        let not_before_ms = now.saturating_add(opts.delay.map(|d| d.as_millis() as u64).unwrap_or(0));

        let rec = WalRecord::Enqueued {
            id,
            payload: payload.to_vec(),
            max_attempts,
            not_before_ms,
            idempotency_key: opts.idempotency_key.clone(),
        };
        // Durable BEFORE publish. [FIX-FSYNC-ROLLBACK]
        g.durable.append(&rec)?;

        // Publish in-memory.
        g.next_id += 1;
        let payload_arc: Arc<[u8]> = Arc::from(payload.to_vec().into_boxed_slice());
        let job = Job {
            id,
            payload: payload_arc,
            attempts: 0,
            max_attempts,
            lease_epoch: 0,
            deliveries: 0,
            status: Status::Ready { not_before_ms },
            not_before_ms,
        };
        g.jobs.insert(id, job);
        g.ready_by_time.push(Reverse((not_before_ms, id, 0)));
        if let Some(key) = opts.idempotency_key {
            g.idem.insert(key, id);
        }

        self.maybe_compact(&mut g)?;
        drop(g);
        self.not_empty.notify_all();
        Ok(id)
    }

    /// Reclaim expired leases + promote due backoffs, then select. Returns the
    /// chosen lease, if any. Runs entirely under the lock.
    fn claim_locked(&self, g: &mut Inner) -> Result<Option<Lease>, JobError> {
        let now_wall = self.clock.now_ms();
        let now_mono = self.clock.mono_ms();

        self.reclaim_sweep(g, now_mono, now_wall)?;

        // Select the earliest-due Ready job.
        loop {
            let top = match g.ready_by_time.peek().copied() {
                Some(Reverse(t)) => t,
                None => return Ok(None),
            };
            let (nb, id, epoch) = top;

            // Validate against jobs[].
            let valid = match g.jobs.get(&id) {
                Some(j) => {
                    matches!(j.status, Status::Ready { not_before_ms } if not_before_ms == nb)
                        && j.lease_epoch == epoch
                }
                None => false,
            };
            if !valid {
                // Stale heap entry. [FIX-STALEHEAP]
                g.ready_by_time.pop();
                continue;
            }
            if nb > now_wall {
                // Earliest is in the future → nothing due. Leave it in place.
                return Ok(None);
            }

            // Due and valid: claim it.
            g.ready_by_time.pop();

            let new_epoch = g.next_epoch;
            // visibility_timeout is a caller-supplied Duration with no upper
            // bound; as_millis() returns u128, so cap at u64::MAX before the
            // cast to avoid silent truncation, then saturating_add so a near-
            // u64::MAX clock or pathologically large timeout can't wrap a
            // deadline back to a small value (which would make the just-claimed
            // lease appear instantly expired and reclaimable). [FIX-VIS-OVERFLOW]
            let vis = u64::try_from(self.cfg.visibility_timeout.as_millis()).unwrap_or(u64::MAX);
            let mono_deadline = now_mono.saturating_add(vis);
            let wall_deadline_ms = now_wall.saturating_add(vis);

            let job = g.jobs.get(&id).unwrap();
            let new_deliveries = job.deliveries + 1;

            // If delivering again would exceed the cap, dead-letter instead of
            // handing out. [FIX-POISON-LOOP]
            if new_deliveries > job.max_attempts {
                let attempts = job.attempts;
                let rec = WalRecord::Failed {
                    id,
                    attempts,
                    outcome: FailedOutcomeRec::Dead,
                };
                g.durable.append(&rec)?;
                let job = g.jobs.get_mut(&id).unwrap();
                job.status = Status::Dead;
                continue;
            }

            let rec = WalRecord::Claimed {
                id,
                epoch: new_epoch,
                wall_deadline_ms,
                deliveries: new_deliveries,
            };
            // Durable BEFORE the worker can start. [FIX-CLAIM-DURABLE]
            g.durable.append(&rec)?;

            // Publish.
            g.next_epoch += 1;
            let job = g.jobs.get_mut(&id).unwrap();
            job.lease_epoch = new_epoch;
            job.deliveries = new_deliveries;
            job.status = Status::Claimed {
                mono_deadline,
                wall_deadline_ms,
            };
            let lease = Lease {
                id,
                epoch: new_epoch,
                payload: job.payload.clone(),
                attempts: job.attempts,
                deadline_ms: wall_deadline_ms,
            };
            g.claimed_by_deadline
                .push(Reverse((mono_deadline, id, new_epoch)));

            self.maybe_compact(g)?;
            return Ok(Some(lease));
        }
    }

    /// Reclaim leases whose monotonic deadline has passed. [FIX-EPOCH-ORDER]
    fn reclaim_sweep(&self, g: &mut Inner, now_mono: u64, now_wall: u64) -> Result<(), JobError> {
        loop {
            let top = match g.claimed_by_deadline.peek().copied() {
                Some(Reverse(t)) => t,
                None => break,
            };
            let (deadline, id, epoch) = top;
            if deadline > now_mono {
                break;
            }
            // Validate.
            let still_claimed = match g.jobs.get(&id) {
                Some(j) => matches!(j.status, Status::Claimed { .. }) && j.lease_epoch == epoch,
                None => false,
            };
            if !still_claimed {
                g.claimed_by_deadline.pop();
                continue;
            }
            g.claimed_by_deadline.pop();

            let new_epoch = g.next_epoch;
            let job = g.jobs.get(&id).unwrap();

            // Poison cap on reclaim too. [FIX-POISON-LOOP]
            if job.deliveries >= job.max_attempts {
                let attempts = job.attempts;
                let rec = WalRecord::Failed {
                    id,
                    attempts,
                    outcome: FailedOutcomeRec::Dead,
                };
                g.durable.append(&rec)?;
                let job = g.jobs.get_mut(&id).unwrap();
                job.status = Status::Dead;
                continue;
            }

            // Bump epoch durably BEFORE the job becomes claimable. [FIX-EPOCH-ORDER]
            let rec = WalRecord::Reclaimed {
                id,
                epoch: new_epoch,
            };
            g.durable.append(&rec)?;

            g.next_epoch += 1;
            let nb = now_wall.max(job.not_before_ms);
            let job = g.jobs.get_mut(&id).unwrap();
            job.lease_epoch = new_epoch;
            job.status = Status::Ready { not_before_ms: nb };
            job.not_before_ms = nb;
            g.ready_by_time.push(Reverse((nb, id, new_epoch)));
        }
        Ok(())
    }

    pub fn claim(&self) -> Result<Option<Lease>, JobError> {
        let mut g = self.lock()?;
        let r = self.claim_locked(&mut g)?;
        Ok(r)
    }

    pub fn claim_blocking(&self, timeout: Duration) -> Result<Option<Lease>, JobError> {
        let deadline_mono = self
            .clock
            .mono_ms()
            .saturating_add(timeout.as_millis() as u64);
        let mut g = self.lock()?;
        loop {
            if let Some(lease) = self.claim_locked(&mut g)? {
                return Ok(Some(lease));
            }
            let now_mono = self.clock.mono_ms();
            if now_mono >= deadline_mono {
                return Ok(None);
            }
            let remaining = deadline_mono - now_mono;

            // Bound the wait by the earliest future not_before and the earliest
            // lease deadline so we wake by pure clock passage.
            // [FIX-LOSTWAKE][FIX-STRANDED]
            let mut wait_ms = remaining;
            let now_wall = self.clock.now_ms();
            if let Some(Reverse((nb, _, _))) = g.ready_by_time.peek().copied() {
                if nb > now_wall {
                    let until = nb - now_wall;
                    wait_ms = wait_ms.min(until.max(1));
                } else {
                    wait_ms = 1; // something is due now
                }
            }
            if let Some(Reverse((deadline, _, _))) = g.claimed_by_deadline.peek().copied() {
                if deadline > now_mono {
                    wait_ms = wait_ms.min(deadline - now_mono);
                } else {
                    wait_ms = 1;
                }
            }
            let wait_ms = wait_ms.max(1);

            let (ng, _timeout_res) = self
                .not_empty
                .wait_timeout(g, Duration::from_millis(wait_ms))
                .map_err(|_| JobError::Io(io::Error::new(io::ErrorKind::Other, "lock poisoned")))?;
            g = ng;
        }
    }

    pub fn complete(&self, id: JobId, epoch: u64) -> Result<(), JobError> {
        let mut g = self.lock()?;
        let job = g.jobs.get(&id).ok_or(JobError::NotFound)?;
        let is_claimed = matches!(job.status, Status::Claimed { .. });
        if !is_claimed || job.lease_epoch != epoch {
            return Err(JobError::StaleLease);
        }
        let rec = WalRecord::Completed { id };
        g.durable.append(&rec)?;
        let job = g.jobs.get_mut(&id).unwrap();
        job.status = Status::Done;
        self.maybe_compact(&mut g)?;
        Ok(())
    }

    pub fn fail(&self, id: JobId, epoch: u64) -> Result<FailOutcome, JobError> {
        let now = self.clock.now_ms();
        let mut g = self.lock()?;
        let job = g.jobs.get(&id).ok_or(JobError::NotFound)?;
        let is_claimed = matches!(job.status, Status::Claimed { .. });
        if !is_claimed || job.lease_epoch != epoch {
            return Err(JobError::StaleLease);
        }

        let new_attempts = job.attempts + 1;
        let max_attempts = job.max_attempts;
        let deliveries = job.deliveries;

        let dead = new_attempts >= max_attempts || deliveries >= max_attempts;

        if dead {
            let rec = WalRecord::Failed {
                id,
                attempts: new_attempts,
                outcome: FailedOutcomeRec::Dead,
            };
            g.durable.append(&rec)?;
            let job = g.jobs.get_mut(&id).unwrap();
            job.attempts = new_attempts;
            job.status = Status::Dead;
            self.maybe_compact(&mut g)?;
            Ok(FailOutcome::DeadLettered)
        } else {
            let delay = self.backoff_delay(&mut g, new_attempts);
            let not_before = now + delay;
            let rec = WalRecord::Failed {
                id,
                attempts: new_attempts,
                outcome: FailedOutcomeRec::Retry {
                    not_before_ms: not_before,
                },
            };
            g.durable.append(&rec)?;

            // Bump epoch on fail so a duplicate stale finalize cannot match.
            let new_epoch = g.next_epoch;
            g.next_epoch += 1;
            let job = g.jobs.get_mut(&id).unwrap();
            job.attempts = new_attempts;
            job.lease_epoch = new_epoch;
            job.status = Status::Ready {
                not_before_ms: not_before,
            };
            job.not_before_ms = not_before;
            g.ready_by_time.push(Reverse((not_before, id, new_epoch)));
            self.maybe_compact(&mut g)?;
            drop(g);
            self.not_empty.notify_all();
            Ok(FailOutcome::Retrying {
                ready_at_ms: not_before,
            })
        }
    }

    /// Compute backoff: min(max, base << (attempts-1)) with saturating shift and
    /// optional +/-12.5% jitter.
    fn backoff_delay(&self, g: &mut Inner, attempts: u32) -> u64 {
        let base = self.cfg.base_backoff.as_millis() as u64;
        let max = self.cfg.max_backoff.as_millis() as u64;
        let shift = attempts.saturating_sub(1);
        let scaled = if shift >= 63 {
            max
        } else {
            base.checked_shl(shift).unwrap_or(u64::MAX).min(max)
        };
        let mut delay = scaled.min(max);
        if self.cfg.jitter && delay > 0 {
            // Deterministic pseudo-jitter in [-12.5%, +12.5%].
            g.jitter_ctr = g
                .jitter_ctr
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1);
            let frac = (g.jitter_ctr >> 40) % 251; // 0..=250
            let signed = frac as i128 - 125; // -125..=125 (per 1000)
            // Use i128 so `delay` (a u64, up to u64::MAX) times `signed`
            // (|signed| <= 125) cannot overflow, and the `delay` value itself
            // is represented exactly rather than wrapping through `i64`.
            let adj = (delay as i128 * signed) / 1000;
            let v = delay as i128 + adj;
            // Re-clamp to [0, max] so jitter never overshoots the configured cap.
            delay = (v.max(0) as u64).min(max);
        }
        delay
    }

    pub fn tick(&self) -> Result<(), JobError> {
        let now_wall = self.clock.now_ms();
        let now_mono = self.clock.mono_ms();
        let mut g = self.lock()?;
        self.reclaim_sweep(&mut g, now_mono, now_wall)?;
        drop(g);
        self.not_empty.notify_all();
        Ok(())
    }

    pub fn stats(&self) -> Stats {
        let g = match self.lock() {
            Ok(g) => g,
            Err(_) => return Stats::default(),
        };
        let mut s = Stats::default();
        for job in g.jobs.values() {
            s.total += 1;
            match job.status {
                Status::Ready { .. } => s.ready += 1,
                Status::Claimed { .. } => s.claimed += 1,
                Status::Done => s.done += 1,
                Status::Dead => s.dead += 1,
            }
        }
        s
    }

    /// Test accessor: the `not_before_ms` of a job (for backoff assertions).
    pub fn job_not_before_ms(&self, id: JobId) -> Option<u64> {
        let g = self.lock().ok()?;
        g.jobs.get(&id).map(|j| j.not_before_ms)
    }

    /// Test accessor: the current lease_epoch of a job.
    pub fn job_epoch(&self, id: JobId) -> Option<u64> {
        let g = self.lock().ok()?;
        g.jobs.get(&id).map(|j| j.lease_epoch)
    }

    /// Test accessor: whether a job is Dead.
    pub fn is_dead(&self, id: JobId) -> bool {
        match self.lock() {
            Ok(g) => g
                .jobs
                .get(&id)
                .map(|j| matches!(j.status, Status::Dead))
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Test accessor: whether a job is Done.
    pub fn is_done(&self, id: JobId) -> bool {
        match self.lock() {
            Ok(g) => g
                .jobs
                .get(&id)
                .map(|j| matches!(j.status, Status::Done))
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Trigger compaction if the threshold is reached. Must be called under the
    /// lock.
    fn maybe_compact(&self, g: &mut Inner) -> Result<(), JobError> {
        if self.cfg.compact_threshold_records == 0 {
            return Ok(());
        }
        if g.durable.appended_records < self.cfg.compact_threshold_records {
            return Ok(());
        }
        self.compact(g)
    }

    /// Force a compaction now (tests/ops).
    pub fn compact_now(&self) -> Result<(), JobError> {
        let mut g = self.lock()?;
        self.compact(&mut g)
    }

    fn compact(&self, g: &mut Inner) -> Result<(), JobError> {
        // The snapshot captures all state implied by WAL records strictly BELOW
        // `snap_offset` (an absolute byte offset in wal.log). We persist the
        // snapshot, then append a `Snapshotted{up_to_offset = snap_offset}`
        // marker. We DO NOT truncate the WAL to empty, and we keep byte offsets
        // ABSOLUTE so a crash at any point leaves a consistent (snapshot + full
        // WAL) pair: on recovery, records below `up_to_offset` are skipped
        // (superseded by the snapshot) and records at/after it are replayed.
        // [FIX-TRUNCATE-PREFIX]
        //
        // Physical space reclamation (rewriting the file to just the suffix)
        // happens crash-safely at the next `open()`, where nothing else runs.
        let snap_offset = g.durable.len;

        // 1. Write snapshot.json atomically (tmp + sync + rename + dir-fsync).
        let snapshot = recovery::Snapshot::from_inner(g, snap_offset);
        recovery::write_snapshot(&self.dir, &snapshot)?;

        // 2. Append the Snapshotted marker durably (offsets stay absolute).
        g.durable.append(&WalRecord::Snapshotted {
            up_to_offset: snap_offset,
        })?;

        g.durable.appended_records = 0;
        Ok(())
    }
}
