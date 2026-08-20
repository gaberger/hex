//! Recovery, snapshotting, and WAL suffix rewriting.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::clock::Clock;
use crate::durable::{Durable, FaultInjector};
use crate::wal::{read_frame, FailedOutcomeRec, FrameRead, WalRecord};
use crate::{Config, Inner, Job, JobError, Status};

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct SnapJob {
    pub id: u64,
    pub payload: Vec<u8>,
    pub attempts: u32,
    pub max_attempts: u32,
    pub lease_epoch: u64,
    pub deliveries: u32,
    pub not_before_ms: u64,
    /// "ready" | "claimed" | "done" | "dead"
    pub status: String,
}

#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub up_to_offset: u64,
    pub next_id: u64,
    pub next_epoch: u64,
    pub jobs: Vec<SnapJob>,
    pub idem: Vec<(String, u64)>,
}

impl Snapshot {
    pub fn from_inner(g: &Inner, up_to_offset: u64) -> Self {
        let jobs = g
            .jobs
            .values()
            .map(|j| SnapJob {
                id: j.id,
                payload: j.payload.to_vec(),
                attempts: j.attempts,
                max_attempts: j.max_attempts,
                lease_epoch: j.lease_epoch,
                deliveries: j.deliveries,
                not_before_ms: j.not_before_ms,
                status: match j.status {
                    Status::Ready { .. } => "ready",
                    // A snapshot taken while a job is claimed records it as
                    // claimed; recovery normalization will re-offer it.
                    Status::Claimed { .. } => "claimed",
                    Status::Done => "done",
                    Status::Dead => "dead",
                }
                .to_string(),
            })
            .collect();
        let idem = g.idem.iter().map(|(k, v)| (k.clone(), *v)).collect();
        Snapshot {
            up_to_offset,
            next_id: g.next_id,
            next_epoch: g.next_epoch,
            jobs,
            idem,
        }
    }
}

pub fn write_snapshot(dir: &Path, snap: &Snapshot) -> Result<(), JobError> {
    let tmp = dir.join("snapshot.json.tmp");
    let final_path = dir.join("snapshot.json");
    let bytes = serde_json::to_vec(snap)
        .map_err(|e| JobError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?;
    {
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(&bytes)?;
        f.flush()?;
        f.sync_data()?;
    }
    std::fs::rename(&tmp, &final_path)?;
    // fsync the directory so the rename is durable.
    let d = File::open(dir)?;
    d.sync_all()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Replay state (folded from snapshot + WAL)
// ---------------------------------------------------------------------------

struct ReplayJob {
    payload: Vec<u8>,
    attempts: u32,
    max_attempts: u32,
    lease_epoch: u64,
    deliveries: u32,
    not_before_ms: u64,
    status: RStatus,
}

#[derive(Clone, Copy, PartialEq)]
enum RStatus {
    Ready,
    Claimed,
    Done,
    Dead,
}

pub fn recover(
    dir: &Path,
    _cfg: &Config,
    clock: &dyn Clock,
    fault: Arc<FaultInjector>,
) -> Result<Inner, JobError> {
    let mut jobs: HashMap<u64, ReplayJob> = HashMap::new();
    let mut idem: HashMap<String, u64> = HashMap::new();
    let mut next_id: u64 = 0;
    let mut next_epoch: u64 = 0;
    let mut skip_below: u64 = 0;

    // 1. Load snapshot if present.
    let snap_path = dir.join("snapshot.json");
    if snap_path.exists() {
        let mut bytes = Vec::new();
        File::open(&snap_path)?.read_to_end(&mut bytes)?;
        match serde_json::from_slice::<Snapshot>(&bytes) {
            Ok(snap) => {
                skip_below = snap.up_to_offset;
                next_id = next_id.max(snap.next_id);
                next_epoch = next_epoch.max(snap.next_epoch);
                for sj in snap.jobs {
                    let status = match sj.status.as_str() {
                        "ready" => RStatus::Ready,
                        "claimed" => RStatus::Claimed,
                        "done" => RStatus::Done,
                        "dead" => RStatus::Dead,
                        _ => RStatus::Ready,
                    };
                    next_id = next_id.max(sj.id + 1);
                    next_epoch = next_epoch.max(sj.lease_epoch + 1);
                    jobs.insert(
                        sj.id,
                        ReplayJob {
                            payload: sj.payload,
                            attempts: sj.attempts,
                            max_attempts: sj.max_attempts,
                            lease_epoch: sj.lease_epoch,
                            deliveries: sj.deliveries,
                            not_before_ms: sj.not_before_ms,
                            status,
                        },
                    );
                }
                for (k, v) in snap.idem {
                    idem.insert(k, v);
                }
            }
            Err(_) => {
                // Corrupt snapshot: ignore it and rebuild purely from WAL.
                skip_below = 0;
            }
        }
    }

    // 2. Replay wal.log.
    let wal_path = dir.join("wal.log");
    let mut data = Vec::new();
    if wal_path.exists() {
        File::open(&wal_path)?.read_to_end(&mut data)?;
    }

    let mut offset: usize = 0;
    while offset < data.len() {
        let frame_offset = offset as u64;
        let slice = &data[offset..];
        match read_frame(slice) {
            FrameRead::Ok { rec, consumed } => {
                // Only fold records at/after the snapshot offset (others are
                // superseded by the snapshot).
                if frame_offset >= skip_below {
                    fold_record(&mut jobs, &mut idem, &mut next_id, &mut next_epoch, rec, &mut skip_below);
                } else if let WalRecord::Snapshotted { up_to_offset } = rec {
                    // Honor snapshot markers even below the threshold.
                    skip_below = skip_below.max(up_to_offset);
                }
                offset += consumed;
            }
            FrameRead::Incomplete | FrameRead::BadCrc => {
                // A frame here is unreadable: either its declared length runs
                // past EOF (Incomplete) or its CRC — which now covers the length
                // prefix [FIX-LEN-CRC] — does not match (BadCrc). In BOTH cases
                // the declared length itself may be the thing that is corrupt,
                // so we MUST NOT trust it to decide torn-tail-vs-interior.
                //
                // Decide by scanning the *rest of the file* for a recoverable
                // frame, independent of this frame's length. If any well-formed
                // frame exists at any later offset, then committed records were
                // written after this point and silently truncating here would be
                // data loss → fail loud. Only if nothing recoverable follows is
                // this a genuine torn tail. [FIX-TORN][FIX-INTERIOR-CORRUPT]
                if has_recoverable_frame_after(&data, offset) {
                    return Err(JobError::Corrupt(format!(
                        "unreadable frame at offset {frame_offset} (interior corruption); \
                         recoverable frame(s) follow — refusing to truncate"
                    )));
                }
                // Genuine torn tail. Drop and stop. [FIX-TORN]
                break;
            }
        }
    }

    // 3. Build the durable writer. Its length is the offset of the last good
    //    frame we accepted (so a torn tail is logically dropped; new appends
    //    overwrite it). We truncate the physical file to that length.
    let good_len = offset as u64;
    if wal_path.exists() && good_len < data.len() as u64 {
        // Truncate away the torn tail so future appends are clean.
        let f = OpenOptions::new().write(true).open(&wal_path)?;
        f.set_len(good_len)?;
        f.sync_data()?;
    }
    let mut durable = Durable::open(dir, good_len, fault)?;

    // 4. Normalization pass: every Claimed job → Ready (crash = instant
    //    expiry), bump epoch, append Reclaimed durably. [FIX-EPOCH-RECOVERY]
    //    But if deliveries >= max_attempts, dead-letter instead. [FIX-POISON-LOOP]
    let now = clock.now_ms();
    let mut claimed_ids: Vec<u64> = jobs
        .iter()
        .filter(|(_, j)| j.status == RStatus::Claimed)
        .map(|(id, _)| *id)
        .collect();
    claimed_ids.sort_unstable();

    for id in claimed_ids {
        let (deliveries, max_attempts, attempts, nb) = {
            let j = jobs.get(&id).unwrap();
            (j.deliveries, j.max_attempts, j.attempts, j.not_before_ms)
        };
        if deliveries >= max_attempts {
            // Dead-letter.
            durable.append(&WalRecord::Failed {
                id,
                attempts,
                outcome: FailedOutcomeRec::Dead,
            })?;
            let j = jobs.get_mut(&id).unwrap();
            j.status = RStatus::Dead;
        } else {
            let new_epoch = next_epoch;
            next_epoch += 1;
            durable.append(&WalRecord::Reclaimed {
                id,
                epoch: new_epoch,
            })?;
            let j = jobs.get_mut(&id).unwrap();
            j.lease_epoch = new_epoch;
            j.status = RStatus::Ready;
            j.not_before_ms = now.max(nb);
        }
    }

    // 5. Build Inner.
    let mut final_jobs: HashMap<u64, Job> = HashMap::new();
    let mut ready_by_time: BinaryHeap<Reverse<(u64, u64, u64)>> = BinaryHeap::new();
    for (id, rj) in jobs {
        let status = match rj.status {
            RStatus::Ready => Status::Ready {
                not_before_ms: rj.not_before_ms,
            },
            // Should not happen after normalization, but keep sound.
            RStatus::Claimed => Status::Ready {
                not_before_ms: rj.not_before_ms,
            },
            RStatus::Done => Status::Done,
            RStatus::Dead => Status::Dead,
        };
        if let Status::Ready { not_before_ms } = status {
            ready_by_time.push(Reverse((not_before_ms, id, rj.lease_epoch)));
        }
        let payload: Arc<[u8]> = Arc::from(rj.payload.into_boxed_slice());
        final_jobs.insert(
            id,
            Job {
                id,
                payload,
                attempts: rj.attempts,
                max_attempts: rj.max_attempts,
                lease_epoch: rj.lease_epoch,
                deliveries: rj.deliveries,
                status,
                not_before_ms: rj.not_before_ms,
            },
        );
    }

    Ok(Inner {
        jobs: final_jobs,
        ready_by_time,
        claimed_by_deadline: BinaryHeap::new(),
        next_id,
        next_epoch,
        idem,
        durable,
        jitter_ctr: 0x9E3779B97F4A7C15,
    })
}

/// Decide whether any recoverable (CRC-valid) frame exists at some offset
/// strictly after the unreadable frame starting at `offset`.
///
/// The unreadable frame's own declared length is NOT trusted here — it may be
/// the corrupted field (a torn write or bit-rot landing in the length prefix
/// makes a corrupt interior frame look exactly like a torn tail if you believe
/// its length). Instead we scan every byte position from `offset + 1` to EOF
/// looking for a self-consistent frame: one whose CRC (covering its own length
/// prefix and payload, [FIX-LEN-CRC]) checks out and whose JSON parses.
///
/// If such a frame is found, committed records were written after the
/// corruption point, so truncating at `offset` would silently lose them →
/// caller fails loud. If no recoverable frame exists anywhere after `offset`,
/// the unreadable region is a genuine torn tail and the caller may drop it.
fn has_recoverable_frame_after(data: &[u8], offset: usize) -> bool {
    let start = offset.saturating_add(1);
    let mut o = start;
    // Need at least a full header plus one payload byte to have any frame.
    while o + crate::wal::HEADER_LEN < data.len() {
        if let FrameRead::Ok { .. } = read_frame(&data[o..]) {
            return true;
        }
        o += 1;
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn fold_record(
    jobs: &mut HashMap<u64, ReplayJob>,
    idem: &mut HashMap<String, u64>,
    next_id: &mut u64,
    next_epoch: &mut u64,
    rec: WalRecord,
    skip_below: &mut u64,
) {
    match rec {
        WalRecord::Enqueued {
            id,
            payload,
            max_attempts,
            not_before_ms,
            idempotency_key,
        } => {
            *next_id = (*next_id).max(id + 1);
            if let Some(k) = idempotency_key {
                idem.insert(k, id);
            }
            jobs.insert(
                id,
                ReplayJob {
                    payload,
                    attempts: 0,
                    max_attempts,
                    lease_epoch: 0,
                    deliveries: 0,
                    not_before_ms,
                    status: RStatus::Ready,
                },
            );
        }
        WalRecord::Claimed {
            id,
            epoch,
            wall_deadline_ms: _,
            deliveries,
        } => {
            *next_epoch = (*next_epoch).max(epoch + 1);
            if let Some(j) = jobs.get_mut(&id) {
                j.lease_epoch = epoch;
                j.deliveries = deliveries;
                j.status = RStatus::Claimed;
            }
        }
        WalRecord::Reclaimed { id, epoch } => {
            *next_epoch = (*next_epoch).max(epoch + 1);
            if let Some(j) = jobs.get_mut(&id) {
                j.lease_epoch = epoch;
                j.status = RStatus::Ready;
            }
        }
        WalRecord::Completed { id } => {
            if let Some(j) = jobs.get_mut(&id) {
                j.status = RStatus::Done;
            }
        }
        WalRecord::Failed {
            id,
            attempts,
            outcome,
        } => {
            if let Some(j) = jobs.get_mut(&id) {
                j.attempts = attempts;
                match outcome {
                    FailedOutcomeRec::Retry { not_before_ms } => {
                        j.status = RStatus::Ready;
                        j.not_before_ms = not_before_ms;
                    }
                    FailedOutcomeRec::Dead => {
                        j.status = RStatus::Dead;
                    }
                }
            }
        }
        WalRecord::Snapshotted { up_to_offset } => {
            *skip_below = (*skip_below).max(up_to_offset);
        }
    }
}
