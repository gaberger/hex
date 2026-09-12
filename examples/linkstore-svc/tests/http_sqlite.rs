//! The gate. A real file on disk, a real TCP port, real HTTP requests.
//!
//! Every test gets its own temporary directory and its own operating-system
//! chosen port, so `cargo test` may run them all at once with nothing shared.

use linkstore_svc::ports::{
    BookmarkStore, Clock, NormalisedUrlValue, TagValue, TimestampValue, TitleValue,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};

// ── The harness ──────────────────────────────────────────────────────────

/// A clock that never moves, so a test can assert on an exact timestamp.
struct FixedClock(i64);

impl Clock for FixedClock {
    fn now(&self) -> TimestampValue {
        TimestampValue::from_millis(self.0)
    }
}

/// A clock that moves one step each time it is read, so saves have an order.
struct SteppingClock {
    next: AtomicI64,
    step: i64,
}

impl Clock for SteppingClock {
    fn now(&self) -> TimestampValue {
        TimestampValue::from_millis(self.next.fetch_add(self.step, Ordering::SeqCst))
    }
}

/// A running service, and the address to reach it on.
struct Running {
    base: String,
    server: JoinHandle<()>,
}

impl Running {
    fn at(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// Stop answering. Used by the restart test.
    fn stop(self) {
        self.server.abort();
    }
}

fn temp_db() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let db = dir.path().join("links.db");
    (dir, db)
}

/// Open the database, bind a free port, and start answering.
async fn start(db: &Path, clock: Arc<dyn Clock>) -> Running {
    // The migration runs here.
    let api = linkstore_svc::bookmark_service(db, clock).expect("open the database");

    // Port zero asks the operating system for a port nobody is using.
    let listener = TcpListener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind a port");
    // Read the address, then hand over **this very listener**. Reading the
    // number, dropping the socket and rebinding leaves a gap another process
    // can take. Handing the live socket over also removes the sleep people add
    // to paper over the race: the operating system is already accepting
    // connections before the server task starts.
    let base = format!("http://{}", listener.local_addr().expect("a local address"));

    let server = tokio::spawn(async move {
        let _ = linkstore_svc::serve(listener, api).await;
    });
    Running { base, server }
}

async fn start_fixed(db: &Path, millis: i64) -> Running {
    start(db, Arc::new(FixedClock(millis))).await
}

/// Ask the file on disk directly, with a second connection on the same path.
///
/// The same **path**, never a copy of the `.db` file: a committed row can
/// still be living in the `-wal` side file, so a lone copy shows nothing and
/// the assertion fails against correct code.
fn count_rows(db: &Path, sql: &str) -> i64 {
    let conn = Connection::open(db).expect("open the file directly");
    conn.query_row(sql, [], |row| row.get(0)).expect("read a count")
}

fn bookmark_rows(db: &Path) -> i64 {
    count_rows(db, "SELECT count(*) FROM bookmarks")
}

fn tag_rows(db: &Path) -> i64 {
    count_rows(db, "SELECT count(*) FROM bookmark_tags")
}

fn body_of(value: &Value, field: &str) -> String {
    value[field].as_str().unwrap_or_default().to_string()
}

// ── T1 ───────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn create_get_list_delete_round_trip() {
    let (_dir, db) = temp_db();
    let service = start_fixed(&db, 1_700_000_000_123).await;
    let http = reqwest::Client::new();

    let posted = http
        .post(service.at("/bookmarks"))
        .json(&json!({
            "url": "https://Example.com/Guide?utm_source=news&id=7",
            "title": "The guide",
            "tags": ["Rust", "sqlite"]
        }))
        .send()
        .await
        .expect("post");
    assert_eq!(posted.status().as_u16(), 201);
    let location = posted
        .headers()
        .get("location")
        .expect("a Location header")
        .to_str()
        .expect("readable header")
        .to_string();
    let created: Value = posted.json().await.expect("json");
    let id = body_of(&created, "id");

    assert_eq!(location, format!("/bookmarks/{id}"));
    assert_eq!(body_of(&created, "url"), "https://example.com/Guide?id=7");
    assert_eq!(body_of(&created, "title"), "The guide");
    assert_eq!(created["created_at"].as_i64(), Some(1_700_000_000_123));
    assert_eq!(created["tags"], json!(["rust", "sqlite"]));

    let fetched = http.get(service.at(&format!("/bookmarks/{id}"))).send().await.expect("get");
    assert_eq!(fetched.status().as_u16(), 200);
    let one: Value = fetched.json().await.expect("json");
    assert_eq!(one, created);

    let listed = http.get(service.at("/bookmarks?tag=rust")).send().await.expect("list");
    assert_eq!(listed.status().as_u16(), 200);
    let many: Value = listed.json().await.expect("json");
    assert_eq!(many.as_array().expect("an array").len(), 1);
    assert_eq!(body_of(&many[0], "id"), id);

    // The row is really on the disk, not just in the reply.
    assert_eq!(bookmark_rows(&db), 1);
    assert_eq!(tag_rows(&db), 2);

    let removed = http.delete(service.at(&format!("/bookmarks/{id}"))).send().await.expect("delete");
    assert_eq!(removed.status().as_u16(), 204);

    let gone = http.get(service.at(&format!("/bookmarks/{id}"))).send().await.expect("get");
    assert_eq!(gone.status().as_u16(), 404);

    // The cascade fired: no orphan tag rows are left behind.
    assert_eq!(bookmark_rows(&db), 0);
    assert_eq!(tag_rows(&db), 0);
}

// ── T2 ───────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn unknown_id_is_404() {
    let (_dir, db) = temp_db();
    let service = start_fixed(&db, 1).await;
    let http = reqwest::Client::new();

    // A well-shaped identity that was never created. Without GROUP BY, the
    // query would answer one row of NULLs here and this would be a 200.
    let never = "3f2504e0-4f89-41d3-9a0c-0305e82c3301";
    let response = http.get(service.at(&format!("/bookmarks/{never}"))).send().await.expect("get");
    assert_eq!(response.status().as_u16(), 404);
    let body: Value = response.json().await.expect("json");
    assert_eq!(body_of(&body, "error"), "not found");

    // Nonsense is missing, not broken. No 500, and no panic.
    let response = http.get(service.at("/bookmarks/not-a-uuid")).send().await.expect("get");
    assert_eq!(response.status().as_u16(), 404);
}

// ── T3 ───────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn delete_is_idempotent() {
    let (_dir, db) = temp_db();
    let service = start_fixed(&db, 1).await;
    let http = reqwest::Client::new();

    let created: Value = http
        .post(service.at("/bookmarks"))
        .json(&json!({"url": "https://x.com/a", "title": "A", "tags": ["t"]}))
        .send()
        .await
        .expect("post")
        .json()
        .await
        .expect("json");
    let id = body_of(&created, "id");

    for _ in 0..2 {
        let response =
            http.delete(service.at(&format!("/bookmarks/{id}"))).send().await.expect("delete");
        assert_eq!(response.status().as_u16(), 204);
    }

    let response = http.delete(service.at("/bookmarks/not-a-uuid")).send().await.expect("delete");
    assert_eq!(response.status().as_u16(), 204);
}

// ── T4 ───────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn fifty_concurrent_posts_same_url_different_tags() {
    let (_dir, db) = temp_db();
    let service = start_fixed(&db, 5_000).await;
    let http = reqwest::Client::new();

    // Different tags on every request is what makes this test mean something.
    // With the same tags, a store that throws tags away still passes.
    let mut work = JoinSet::new();
    for index in 0..50 {
        let http = http.clone();
        let url = service.at("/bookmarks");
        work.spawn(async move {
            let response = http
                .post(url)
                .json(&json!({
                    "url": "https://x.com/hot",
                    "title": "Hot",
                    "tags": [format!("tag-{index:02}")]
                }))
                .send()
                .await
                .expect("post");
            let status = response.status().as_u16();
            let body: Value = response.json().await.expect("json");
            (status, body["id"].as_str().unwrap_or_default().to_string())
        });
    }

    let mut ids = Vec::new();
    while let Some(done) = work.join_next().await {
        let (status, id) = done.expect("the task finished");
        assert_eq!(status, 201, "a repeated save still answers 201");
        ids.push(id);
    }

    assert_eq!(ids.len(), 50);
    assert!(ids.iter().all(|id| id == &ids[0]), "all fifty share one identity");

    // One link, and every tag kept. Tags merge; they are never overwritten.
    assert_eq!(bookmark_rows(&db), 1);
    assert_eq!(tag_rows(&db), 50);
}

// ── T5 ───────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn fifty_concurrent_posts_different_urls() {
    let (_dir, db) = temp_db();
    let service = start_fixed(&db, 5_000).await;
    let http = reqwest::Client::new();

    let mut work = JoinSet::new();
    for index in 0..50 {
        let writer = http.clone();
        let post_url = service.at("/bookmarks");
        work.spawn(async move {
            writer
                .post(post_url)
                .json(&json!({
                    "url": format!("https://x.com/page/{index}"),
                    "title": format!("Page {index}"),
                    "tags": ["bulk"]
                }))
                .send()
                .await
                .expect("post")
                .status()
                .as_u16()
        });

        // Reads interleaved with writes: this is what exhausts a reader pool
        // that answers an error instead of waiting in line.
        let reader = http.clone();
        let list_url = service.at("/bookmarks?tag=bulk");
        work.spawn(async move {
            reader.get(list_url).send().await.expect("list").status().as_u16()
        });
    }

    while let Some(done) = work.join_next().await {
        let status = done.expect("the task finished");
        assert!(status < 500, "no request may fail with {status}");
    }

    assert_eq!(bookmark_rows(&db), 50);
}

// ── T6 ───────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn data_survives_a_restart() {
    let (_dir, db) = temp_db();
    let first = start_fixed(&db, 42).await;
    let http = reqwest::Client::new();

    let created: Value = http
        .post(first.at("/bookmarks"))
        .json(&json!({"url": "https://x.com/keep", "title": "Keep", "tags": ["t"]}))
        .send()
        .await
        .expect("post")
        .json()
        .await
        .expect("json");
    let id = body_of(&created, "id");
    first.stop();

    // A second service on the same file. The migration runs again and must
    // change nothing.
    let second = start_fixed(&db, 99).await;
    let response = http.get(second.at(&format!("/bookmarks/{id}"))).send().await.expect("get");
    assert_eq!(response.status().as_u16(), 200);
    let again: Value = response.json().await.expect("json");
    assert_eq!(body_of(&again, "url"), "https://x.com/keep");
    assert_eq!(again["created_at"].as_i64(), Some(42));

    let version = count_rows(&db, "PRAGMA user_version");
    assert_eq!(version, 1, "the migration ran twice and moved the version");
    assert_eq!(bookmark_rows(&db), 1);
}

// ── T7 ───────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn wal_and_foreign_keys_are_really_on() {
    let (_dir, db) = temp_db();
    let _service = start_fixed(&db, 1).await;

    let conn = Connection::open(&db).expect("open the file directly");

    // The journal mode is written into the file header, so a fresh connection
    // reading `wal` proves the pragma really took on the store's connections.
    let mode: String =
        conn.query_row("PRAGMA journal_mode", [], |row| row.get(0)).expect("read the mode");
    assert_eq!(mode, "wal");

    // Foreign keys are a per-connection setting, so this reading proves the
    // build default, not the store's pragma. The store's own setting is proved
    // behaviourally by the cascade assertion in the round-trip test.
    let foreign_keys: i64 =
        conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0)).expect("read the setting");
    assert_eq!(foreign_keys, 1);
}

// ── T8 ───────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn bad_input_is_400() {
    let (_dir, db) = temp_db();
    let service = start_fixed(&db, 1).await;
    let http = reqwest::Client::new();

    let refused = [
        json!({"url": "javascript:alert(1)", "title": "X", "tags": []}),
        json!({"url": "https://x.com/a", "title": "   ", "tags": []}),
        json!({"url": "https://x.com/a", "title": "X", "tags": ["a".repeat(65)]}),
    ];

    for body in refused {
        let response =
            http.post(service.at("/bookmarks")).json(&body).send().await.expect("post");
        assert_eq!(response.status().as_u16(), 400, "should refuse {body}");
        let answer: Value = response.json().await.expect("json");
        // This adapter's own shape, with a reason a person can read.
        assert!(!body_of(&answer, "error").is_empty());
    }

    assert_eq!(bookmark_rows(&db), 0);
}

// ── T9 ───────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn list_without_a_tag_is_400() {
    let (_dir, db) = temp_db();
    let service = start_fixed(&db, 1).await;

    let response =
        reqwest::get(service.at("/bookmarks")).await.expect("list");
    assert_eq!(response.status().as_u16(), 400);
    let body: Value = response.json().await.expect("json");
    assert_eq!(body_of(&body, "error"), "a tag is required");
}

// ── T10 ──────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn list_is_ordered_and_capped() {
    let (_dir, db) = temp_db();
    let clock = Arc::new(SteppingClock { next: AtomicI64::new(1_000), step: 1_000 });
    let service = start(&db, clock).await;
    let http = reqwest::Client::new();

    // Saved one after another, so the newest is the last one saved.
    for index in 0..5 {
        let response = http
            .post(service.at("/bookmarks"))
            .json(&json!({
                "url": format!("https://x.com/n/{index}"),
                "title": format!("N{index}"),
                "tags": ["ordered"]
            }))
            .send()
            .await
            .expect("post");
        assert_eq!(response.status().as_u16(), 201);
    }

    let listed: Value = http
        .get(service.at("/bookmarks?tag=ordered"))
        .send()
        .await
        .expect("list")
        .json()
        .await
        .expect("json");
    let titles: Vec<String> =
        listed.as_array().expect("an array").iter().map(|b| body_of(b, "title")).collect();
    assert_eq!(titles, vec!["N4", "N3", "N2", "N1", "N0"]);

    let capped: Value = http
        .get(service.at("/bookmarks?tag=ordered&limit=2"))
        .send()
        .await
        .expect("list")
        .json()
        .await
        .expect("json");
    assert_eq!(capped.as_array().expect("an array").len(), 2);
}

// ── T11 ──────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn normalisation_reaches_the_disk() {
    let (_dir, db) = temp_db();
    let service = start_fixed(&db, 7).await;
    let http = reqwest::Client::new();

    let noisy: Value = http
        .post(service.at("/bookmarks"))
        .json(&json!({
            "url": "HTTPS://Example.COM:443/Path?utm_source=x",
            "title": "One",
            "tags": ["t"]
        }))
        .send()
        .await
        .expect("post")
        .json()
        .await
        .expect("json");

    let plain: Value = http
        .post(service.at("/bookmarks"))
        .json(&json!({"url": "https://example.com/Path", "title": "Two", "tags": ["t"]}))
        .send()
        .await
        .expect("post")
        .json()
        .await
        .expect("json");

    assert_eq!(body_of(&noisy, "url"), "https://example.com/Path");
    assert_eq!(body_of(&noisy, "id"), body_of(&plain, "id"));
    assert_eq!(bookmark_rows(&db), 1);
}

// ── T12 ──────────────────────────────────────────────────────────────────

/// Two plain threads driving the store straight through its port, with no web
/// server in the way. This is the write lock and the immediate transaction on
/// their own.
#[test]
fn two_threads_writing_the_same_store() {
    let (_dir, db) = temp_db();
    let store = linkstore_svc::bookmark_store(&db).expect("open the database");
    let tag = TagValue::parse("shared").expect("a tag");

    let mut threads = Vec::new();
    for worker in 0..2 {
        let store: Arc<dyn BookmarkStore> = Arc::clone(&store);
        let tag = tag.clone();
        threads.push(std::thread::spawn(move || {
            for index in 0..25 {
                let url = NormalisedUrlValue::parse(&format!(
                    "https://x.com/w/{worker}/{index}"
                ))
                .expect("a url");
                let title = TitleValue::parse(&format!("W{worker}-{index}")).expect("a title");
                store
                    .upsert(&url, &title, &[tag.clone()], TimestampValue::from_millis(1))
                    .expect("the write succeeded");
            }
        }));
    }
    for thread in threads {
        thread.join().expect("the thread finished");
    }

    assert_eq!(store.list_by_tag(&tag, 500).expect("list").len(), 50);
    assert_eq!(bookmark_rows(&db), 50);
    assert_eq!(tag_rows(&db), 50);
}
