//! Opening connections, and handing them out.
//!
//! One function opens every connection — readers and the writer alike — so a
//! setting can never be true on one connection and false on another.

use crate::ports::StoreError;
use rusqlite::Connection;
use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// How many readers work at once. WAL lets readers run while the writer
/// writes, so this is real parallelism and not a queue in disguise.
const READERS: usize = 4;

/// How long a connection waits for a lock before it gives up.
const BUSY_TIMEOUT: Duration = Duration::from_millis(5_000);

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        StoreError::Unavailable(error.to_string())
    }
}

/// Open one connection and prove its settings really took.
///
/// The order of these lines is the whole function. Two of them fail in silence
/// if they run in the wrong place.
pub fn open_connection(path: &Path) -> Result<Connection, StoreError> {
    let conn = Connection::open(path)?;

    // First, always. `PRAGMA journal_mode = WAL` can be refused by a live
    // writer. Without a busy timeout already set, this connection quietly
    // stays on the old journal, then blocks the writer it was meant to free.
    conn.busy_timeout(BUSY_TIMEOUT)?;

    // Read the answer back. The pragma returns the mode it actually reached,
    // so believing it without looking is how a gate degrades in silence.
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::Unavailable(format!("journal_mode is {mode}")));
    }

    // `PRAGMA foreign_keys` does nothing inside a transaction, and SQLite
    // reports no error when it is ignored. Set it before any transaction
    // starts, then read it back — otherwise ON DELETE CASCADE never fires and
    // orphan tag rows pile up forever.
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
    let foreign_keys: i64 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    if foreign_keys != 1 {
        return Err(StoreError::Unavailable("foreign_keys off".into()));
    }

    Ok(conn)
}

/// Recover a poisoned lock instead of spreading the panic.
///
/// One bad request must not kill the service. This is only safe because every
/// write goes through a `Transaction` guard, which rolls back when it drops —
/// including while a thread is panicking. A connection is therefore never left
/// half inside a transaction for the next caller to trip over.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One writer and a small set of readers, all opened the same way.
pub struct ConnectionPool {
    writer: Mutex<Connection>,
    readers: Mutex<Vec<Connection>>,
    a_reader_is_free: Condvar,
}

impl ConnectionPool {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let writer = open_connection(path)?;
        let mut readers = Vec::with_capacity(READERS);
        for _ in 0..READERS {
            readers.push(open_connection(path)?);
        }
        Ok(ConnectionPool {
            writer: Mutex::new(writer),
            readers: Mutex::new(readers),
            a_reader_is_free: Condvar::new(),
        })
    }

    /// Take the one writer.
    ///
    /// This lock is not overhead. SQLite serialises writes whatever you do —
    /// without the lock, threads collide inside SQLite, get told the file is
    /// busy, retry, and collide again. A queue replaces a scramble.
    pub fn writer(&self) -> MutexGuard<'_, Connection> {
        lock(&self.writer)
    }

    /// Borrow a reader, waiting in line if all of them are out.
    ///
    /// Waiting is the point. An empty pool that returns an error makes a
    /// "no failed requests" test pass on a fast machine and fail on a slow
    /// one. A queue removes the question.
    pub fn reader(&self) -> ReaderGuard<'_> {
        let mut readers = lock(&self.readers);
        while readers.is_empty() {
            readers = self
                .a_reader_is_free
                .wait(readers)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        let conn = readers.pop();
        drop(readers); // The lock is never held while SQLite works.
        ReaderGuard { pool: self, conn }
    }
}

/// A borrowed reader. It goes back to the pool when it drops.
pub struct ReaderGuard<'a> {
    pool: &'a ConnectionPool,
    conn: Option<Connection>,
}

impl Deref for ReaderGuard<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn.as_ref().expect("a borrowed reader always holds a connection")
    }
}

impl DerefMut for ReaderGuard<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.conn.as_mut().expect("a borrowed reader always holds a connection")
    }
}

impl Drop for ReaderGuard<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            lock(&self.pool.readers).push(conn);
            self.pool.a_reader_is_free.notify_one();
        }
    }
}
