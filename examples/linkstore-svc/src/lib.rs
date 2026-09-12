//! linkstore-svc — a hexagonal skeleton that runs.
//!
//! `lib.rs` is the **composition root**: the only place allowed to name a
//! concrete adapter. Everything else depends on the port.
//!
//! ```text
//!   domain  ←  ports  ←  usecases
//!                ↑
//!           adapters (secondary)
//!                ↑
//!        lib.rs — wires them, once
//! ```
//!
//! Check it with `hex analyze .`.

pub mod adapters;
pub mod domain;
pub mod ports;
pub mod usecases;

use adapters::primary::http;
use adapters::secondary::sqlite_store::SqliteBookmarkStore;
use adapters::secondary::system_clock::SystemClock;
use adapters::secondary::InMemoryCounterStore;
use domain::Count;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::net::TcpListener;
use usecases::bookmarks::BookmarkService;

/// Build the application with its real adapters.
///
/// The one line below is the whole composition decision. Change
/// `InMemoryCounterStore` to a file-backed store and nothing else moves.
pub fn counter() -> impl ports::CounterStore {
    InMemoryCounterStore::default()
}

/// Run the use case against a freshly composed application.
pub fn increment_once() -> Count {
    let mut store = counter();
    usecases::increment(&mut store)
}

// ── The bookmark service ─────────────────────────────────────────────────
//
// Four functions, and between them the only four adapter names in this
// codebase outside their own files.

/// The real wall clock.
pub fn system_clock() -> Arc<dyn ports::Clock> {
    Arc::new(SystemClock)
}

/// Open the file, wire the store and the clock into the use case, and hand
/// back the driving port.
///
/// The clock is a **parameter**, not a choice made in here. If this function
/// picked the system clock itself, no test could ever inject a fake one, and
/// the `Clock` port would be decoration rather than a seam.
pub fn bookmark_service(
    db: &Path,
    clock: Arc<dyn ports::Clock>,
) -> Result<Arc<dyn ports::BookmarkApi>, ports::StoreError> {
    let store = Arc::new(SqliteBookmarkStore::open(db)?);
    Ok(Arc::new(BookmarkService::new(store, clock)))
}

/// Open the file and hand back the driven port on its own.
///
/// This exists so a test can drive the store directly without naming the
/// adapter. Choosing SQLite stays here, in the one file allowed to choose.
pub fn bookmark_store(db: &Path) -> Result<Arc<dyn ports::BookmarkStore>, ports::StoreError> {
    Ok(Arc::new(SqliteBookmarkStore::open(db)?))
}

/// Answer requests on a listener that is already open.
///
/// Taking the live listener, rather than a port number, is what lets a caller
/// ask the operating system for a free port and hand the very same socket
/// over. Reading the number and rebinding leaves a gap another process can
/// take.
pub async fn serve(listener: TcpListener, api: Arc<dyn ports::BookmarkApi>) -> std::io::Result<()> {
    axum::serve(listener, http::router(api)).await
}

/// Open the database, bind the port, and serve until stopped.
pub async fn run(db: &Path, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    // Built from octets. A literal address string is how a tool quietly
    // acquires an address it never decided to have.
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;
    let api = bookmark_service(db, system_clock())?;
    serve(listener, api).await?;
    Ok(())
}
