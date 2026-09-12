//! The real rack of hooks: a SQLite file on disk.
//!
//! This adapter imports `ports/` only. Every domain type it names arrives
//! through the port's re-exports.

use crate::adapters::secondary::sqlite_pool::ConnectionPool;
use crate::ports::{
    BookmarkIdValue, BookmarkStore, BookmarkValue, NormalisedUrlValue, StoreError, TagValue,
    TimestampValue, TitleValue,
};
use rusqlite::{params, OptionalExtension, Row, TransactionBehavior};
use std::path::Path;
use uuid::Uuid;

/// ASCII 31, the unit separator. Tags are joined with it inside SQL.
///
/// A comma would need a domain rule banning commas in tags, which holds only
/// while every writer of this file remembers the rule. `Tag::parse` already
/// rejects control characters, so this separator is unambiguous by
/// construction rather than by discipline.
const UNIT_SEPARATOR: char = '\u{1f}';

const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS bookmarks (
  id          TEXT PRIMARY KEY,
  url         TEXT NOT NULL UNIQUE,
  title       TEXT NOT NULL,
  created_at  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS bookmark_tags (
  bookmark_id TEXT NOT NULL REFERENCES bookmarks(id) ON DELETE CASCADE,
  tag         TEXT NOT NULL,
  PRIMARY KEY (bookmark_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_tags_tag ON bookmark_tags(tag, bookmark_id);
PRAGMA user_version = 1;
";

/// The version of the schema this build understands.
const SCHEMA_VERSION: i64 = 1;

const SELECT_COLUMNS: &str =
    "b.id, b.url, b.title, b.created_at, group_concat(t.tag, char(31))";

/// The identity is a fresh random value, and the URL carries the UNIQUE
/// constraint. `DO UPDATE` rather than `DO NOTHING`, because `DO NOTHING` with
/// `RETURNING` gives back zero rows on a conflict, and the caller then decides
/// the write failed.
///
/// `min()` on `created_at` means the earliest time always wins. A repeated
/// save can never move a bookmark's birthday.
const UPSERT: &str = "
INSERT INTO bookmarks (id, url, title, created_at)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(url) DO UPDATE SET
  title      = excluded.title,
  created_at = min(bookmarks.created_at, excluded.created_at)
RETURNING id
";

pub struct SqliteBookmarkStore {
    pool: ConnectionPool,
}

impl SqliteBookmarkStore {
    /// Open the file and bring its schema up to date. Safe to call again.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let store = SqliteBookmarkStore { pool: ConnectionPool::open(path)? };
        store.migrate()?;
        Ok(store)
    }

    /// Running this twice changes nothing.
    ///
    /// The version is read **inside** the write transaction. Read it outside,
    /// and two processes opening the file together both see version 0 and both
    /// migrate. Here the second one waits for the file lock, reads version 1,
    /// and does nothing. The database's own lock does the work.
    fn migrate(&self) -> Result<(), StoreError> {
        let mut conn = self.pool.writer();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version == 0 {
            tx.execute_batch(SCHEMA_V1)?;
        } else if version > SCHEMA_VERSION {
            // An old build meeting a new file must refuse to touch it.
            return Err(StoreError::Corrupt(format!(
                "database is version {version}, this binary knows {SCHEMA_VERSION}"
            )));
        }
        tx.commit()?;
        Ok(())
    }
}

/// Rebuild a bookmark from one row. Nothing here re-judges the values: a row
/// that was valid when it was written stays readable forever.
fn row_to_bookmark(row: &Row<'_>) -> rusqlite::Result<BookmarkValue> {
    let id: String = row.get(0)?;
    let url: String = row.get(1)?;
    let title: String = row.get(2)?;
    let created_at: i64 = row.get(3)?;
    let joined: Option<String> = row.get(4)?;

    let tags = joined
        .unwrap_or_default()
        .split(UNIT_SEPARATOR)
        .filter(|piece| !piece.is_empty())
        .map(|piece| TagValue::rehydrate(piece.to_string()))
        .collect();

    // `Bookmark::rehydrate` sorts the tags, so the answer never depends on
    // which order this SQLite build happened to concatenate them in.
    Ok(BookmarkValue::rehydrate(
        BookmarkIdValue::rehydrate(id),
        NormalisedUrlValue::rehydrate(url),
        TitleValue::rehydrate(title),
        TimestampValue::rehydrate(created_at),
        tags,
    ))
}

impl BookmarkStore for SqliteBookmarkStore {
    fn upsert(
        &self,
        url: &NormalisedUrlValue,
        title: &TitleValue,
        tags: &[TagValue],
        now: TimestampValue,
    ) -> Result<BookmarkIdValue, StoreError> {
        let mut conn = self.pool.writer();
        // The guard, never the word BEGIN. A `Transaction` rolls back when it
        // drops, including during a panic, so a connection is never left
        // inside a transaction for the next caller. `Immediate` takes the
        // write lock up front: a transaction that reads first and writes later
        // is refused at once, and no busy timeout saves it.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let candidate = Uuid::new_v4().to_string();
        // `query_row`, not `execute`: RETURNING through execute is an error in
        // rusqlite, not a quiet no-op.
        let id: String = tx.query_row(
            UPSERT,
            params![candidate, url.as_str(), title.as_str(), now.millis()],
            |row| row.get(0),
        )?;

        {
            // Tags merge. Tags are never removed here. A title is one value,
            // so replacing it is a choice; a tag set is a collection, so
            // replacing it is a loss — and it would answer 201 while losing.
            let mut statement =
                tx.prepare("INSERT OR IGNORE INTO bookmark_tags (bookmark_id, tag) VALUES (?1, ?2)")?;
            for tag in tags {
                statement.execute(params![id, tag.as_str()])?;
            }
        }

        tx.commit()?;
        Ok(BookmarkIdValue::rehydrate(id))
    }

    fn get(&self, id: &BookmarkIdValue) -> Result<Option<BookmarkValue>, StoreError> {
        let conn = self.pool.reader();
        // `GROUP BY b.id` is not decoration. Without it this is a bare
        // aggregate query, and SQLite answers with exactly one row of NULLs
        // when nothing matches — so `query_row` succeeds, the code reads a
        // NULL id, and the 404 path never runs.
        let sql = format!(
            "SELECT {SELECT_COLUMNS}
             FROM bookmarks b LEFT JOIN bookmark_tags t ON t.bookmark_id = b.id
             WHERE b.id = ?1
             GROUP BY b.id"
        );
        let found = conn
            .query_row(&sql, params![id.as_str()], row_to_bookmark)
            .optional()?;
        Ok(found)
    }

    fn list_by_tag(&self, tag: &TagValue, limit: u32) -> Result<Vec<BookmarkValue>, StoreError> {
        let conn = self.pool.reader();
        // The tiebreak is `rowid`, not `id`. The identity is random, so
        // ordering by it gives arbitrary order wearing a tiebreak costume.
        // `rowid` is insertion order inside the file, so two bookmarks saved
        // in the same millisecond still come back newest-first.
        let sql = format!(
            "SELECT {SELECT_COLUMNS}
             FROM bookmarks b
             JOIN bookmark_tags f ON f.bookmark_id = b.id AND f.tag = ?1
             LEFT JOIN bookmark_tags t ON t.bookmark_id = b.id
             GROUP BY b.id
             ORDER BY b.created_at DESC, b.rowid DESC
             LIMIT ?2"
        );
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(params![tag.as_str(), i64::from(limit)], row_to_bookmark)?;
        let mut found = Vec::new();
        for row in rows {
            found.push(row?);
        }
        Ok(found)
    }

    fn delete(&self, id: &BookmarkIdValue) -> Result<(), StoreError> {
        let mut conn = self.pool.writer();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // The cascade removes the tag rows. One row or none, the goal is met.
        tx.execute("DELETE FROM bookmarks WHERE id = ?1", params![id.as_str()])?;
        tx.commit()?;
        Ok(())
    }
}
