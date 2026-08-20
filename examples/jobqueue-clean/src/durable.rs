//! Durable WAL writer.
//!
//! Owns a raw `File` (NOT a `BufWriter`) and does its own buffering with an
//! explicit `Vec<u8>` so there is no hidden userspace buffer that `sync_data`
//! would skip. [FIX-BUFWRITER]
//!
//! Write protocol per durable commit:
//!   1. write_all(frame)  (single contiguous buffer)
//!   2. flush + sync_data (fdatasync)
//!   3. on first append after create/compaction: fsync the dir [FIX-DIRSYNC]
//!
//! fsync failures roll back: the caller stages in-memory mutation and only
//! publishes after `append` returns Ok. [FIX-FSYNC-ROLLBACK]

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::wal::WalRecord;

/// Test seam: when set true, the next `sync_data` call errors. [FIX-FSYNC-ROLLBACK test]
#[derive(Default)]
pub struct FaultInjector {
    pub fail_sync: AtomicBool,
}

impl FaultInjector {
    pub fn arm_sync_failure(&self) {
        self.fail_sync.store(true, Ordering::SeqCst);
    }
    fn take(&self) -> bool {
        self.fail_sync.swap(false, Ordering::SeqCst)
    }
}

pub struct Durable {
    file: File,
    dir: PathBuf,
    /// Whether the directory entry for wal.log has been fsync'd.
    dir_synced: bool,
    /// Current byte length of wal.log (== next append offset).
    pub len: u64,
    /// Records appended since open (for compaction triggering).
    pub appended_records: u64,
    fault: Arc<FaultInjector>,
}

impl Durable {
    /// Open (creating if needed) the wal.log in append mode. `existing_len` is
    /// the byte length after recovery replay (so `len` is correct).
    pub fn open(
        dir: &Path,
        existing_len: u64,
        fault: Arc<FaultInjector>,
    ) -> io::Result<Self> {
        let path = dir.join("wal.log");
        let already_exists = path.exists();
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&path)?;
        Ok(Self {
            file,
            dir: dir.to_path_buf(),
            // If the file already existed on disk, its dir entry is durable.
            dir_synced: already_exists,
            len: existing_len,
            appended_records: 0,
            fault,
        })
    }

    fn fsync_dir(&self) -> io::Result<()> {
        let dir = File::open(&self.dir)?;
        dir.sync_all()
    }

    /// Append + durably commit one record. Returns the byte offset at which the
    /// frame was written. The in-memory mutation must be published by the caller
    /// only AFTER this returns Ok. [FIX-FSYNC-ROLLBACK]
    pub fn append(&mut self, rec: &WalRecord) -> io::Result<u64> {
        let frame = crate::wal::encode_frame(rec)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let offset = self.len;

        self.file.write_all(&frame)?;
        self.file.flush()?;

        if self.fault.take() {
            // Simulated fsync failure: the bytes may be in the page cache but
            // are NOT durable. We must NOT advance `len` or let the caller
            // publish. Truncate back to keep the file consistent for replay.
            let _ = self.file.set_len(offset);
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "injected sync_data failure",
            ));
        }

        self.file.sync_data()?;

        if !self.dir_synced {
            self.fsync_dir()?;
            self.dir_synced = true;
        }

        self.len += frame.len() as u64;
        self.appended_records += 1;
        Ok(offset)
    }

}
