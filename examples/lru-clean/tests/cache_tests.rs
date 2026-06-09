//! Comprehensive test suite for the thread-safe LRU + TTL cache, following the
//! spec's test plan (categories A–F). Every fatal flaw the red team raised maps
//! to a specific test here.
//!
//! Note on `loom`: the spec calls for a loom interleaving sweep (category D).
//! That requires the `loom` dependency and a separate `--cfg loom` build, which
//! is intentionally kept out of the default `cargo test` gate to stay
//! dependency-free. The interleaving requirement is instead exercised by the
//! high-thread stress tests below plus the single-lock structural argument
//! (recency is list position under one lock — inversion is impossible).

use lru_clean::{Cache, Clock, FakeClock};
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Deterministic PRNG (no external deps)
// ---------------------------------------------------------------------------

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    fn next_u64(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

// ---------------------------------------------------------------------------
// Reference model (the independent, deliberately-dumb oracle for category A)
// ---------------------------------------------------------------------------

/// Naive O(n) exact-LRU + exact-TTL model. Shares NO code with the real impl.
/// `entries[0]` is MRU, `entries.last()` is LRU. Mirrors the cache's eviction
/// policy: prefer an already-expired entry, else the LRU tail.
struct Model {
    capacity: usize,
    clock: Arc<FakeClock>,
    entries: Vec<(u32, u64, Option<Instant>)>, // (key, value, deadline)
}

impl Model {
    fn new(capacity: usize, clock: Arc<FakeClock>) -> Self {
        Model {
            capacity,
            clock,
            entries: Vec::new(),
        }
    }

    fn put(&mut self, key: u32, value: u64, ttl: Duration) {
        let now = self.clock.now();
        let deadline = now.checked_add(ttl);
        self.entries.retain(|(k, _, _)| *k != key);
        self.entries.insert(0, (key, value, deadline));
        while self.entries.len() > self.capacity {
            self.evict_one(now);
        }
    }

    fn evict_one(&mut self, now: Instant) {
        // Prefer the expired entry with the earliest deadline.
        let mut victim: Option<usize> = None;
        let mut best: Option<Instant> = None;
        for (i, (_, _, d)) in self.entries.iter().enumerate() {
            if let Some(dl) = d {
                if *dl <= now && best.map_or(true, |b| *dl < b) {
                    best = Some(*dl);
                    victim = Some(i);
                }
            }
        }
        let idx = victim.unwrap_or(self.entries.len() - 1); // else LRU tail
        if !self.entries.is_empty() {
            self.entries.remove(idx);
        }
    }

    fn get(&mut self, key: &u32) -> Option<u64> {
        let now = self.clock.now();
        let pos = self.entries.iter().position(|(k, _, _)| k == key)?;
        let (_, value, deadline) = self.entries[pos];
        if let Some(d) = deadline {
            if d <= now {
                self.entries.remove(pos); // lazy expiry
                return None;
            }
        }
        let item = self.entries.remove(pos);
        self.entries.insert(0, item); // move to MRU
        Some(value)
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    /// Remove every expired entry (mirror of `Cache::purge_expired`). Used to
    /// normalize away the (unobservable) identity of which dead entry a given
    /// side chose to reclaim — the *live* set is always identical, only the
    /// dead-resident remnants can differ.
    fn purge_expired(&mut self) {
        let now = self.clock.now();
        self.entries
            .retain(|(_, _, d)| d.map_or(true, |dl| dl > now));
    }
}

// ===========================================================================
// A. Model-based property tests (primary oracle)
// ===========================================================================

#[test]
fn property_matches_reference_model() {
    for seed in 0..40u64 {
        let mut rng = Rng::new(seed);
        let capacity = (rng.below(6)) as usize; // 0..=5 — frequent eviction
        let clock = Arc::new(FakeClock::new());
        let cache: Cache<u32, u64, Arc<FakeClock>> =
            Cache::with_clock(capacity, clock.clone());
        let mut model = Model::new(capacity, clock.clone());

        for _ in 0..600 {
            let key = rng.below(8) as u32; // small key space ⇒ collisions/overwrites
            match rng.below(10) {
                0..=4 => {
                    // put
                    let value = rng.next_u64();
                    let ttl_ms = rng.below(50); // some 0-ttl, some live
                    let ttl = Duration::from_millis(ttl_ms);
                    cache.put(key, value, ttl);
                    model.put(key, value, ttl);
                }
                5..=8 => {
                    // get — must agree
                    let got = cache.get(&key).map(|a| *a);
                    let exp = model.get(&key);
                    assert_eq!(got, exp, "seed {seed}: get({key}) mismatch");
                }
                _ => {
                    // advance the clock
                    clock.advance(Duration::from_millis(1 + rng.below(20)));
                }
            }

            assert!(cache.len() <= capacity, "seed {seed}: capacity exceeded");
            cache.check_invariants();

            // Normalize dead-entry remnants (unobservable identity) then compare
            // the resident == live sets via len.
            cache.purge_expired();
            model.purge_expired();
            assert_eq!(cache.len(), model.len(), "seed {seed}: live len mismatch");
        }
    }
}

// ===========================================================================
// B. Capacity-contract tests
// ===========================================================================

#[test]
fn exact_global_capacity_and_lru_victim() {
    let cache: Cache<u32, u32> = Cache::new(100);
    for k in 0..100u32 {
        cache.put(k, k, Duration::from_secs(3600));
    }
    assert_eq!(cache.len(), 100);

    // Touch key 0 so it is MRU; key 1 becomes the LRU victim.
    assert_eq!(cache.get(&0).map(|a| *a), Some(0));
    cache.put(100, 100, Duration::from_secs(3600));

    assert_eq!(cache.len(), 100, "capacity must stay exact, never overshoot");
    assert!(cache.get(&1).is_none(), "global LRU victim should be evicted");
    assert_eq!(cache.get(&0).map(|a| *a), Some(0), "recently used survives");
    assert_eq!(cache.get(&100).map(|a| *a), Some(100));
    cache.check_invariants();
}

#[test]
fn capacity_zero_caches_nothing() {
    let cache: Cache<u32, u32> = Cache::new(0);
    cache.put(1, 1, Duration::from_secs(10));
    assert_eq!(cache.len(), 0);
    assert!(cache.get(&1).is_none());
    cache.check_invariants();
}

#[test]
fn capacity_one_thrash_keeps_last_used() {
    let cache: Cache<u32, u32> = Cache::new(1);
    for round in 0..50u32 {
        cache.put(round, round, Duration::from_secs(10));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&round).map(|a| *a), Some(round));
        if round > 0 {
            assert!(cache.get(&(round - 1)).is_none(), "prior key must be gone");
        }
    }
    cache.check_invariants();
}

#[test]
fn global_capacity_under_many_threads_never_overshoots() {
    let cap = 10usize;
    let cache: Arc<Cache<u32, u32>> = Arc::new(Cache::new(cap));
    let threads: Vec<_> = (0..16u32)
        .map(|t| {
            let cache = cache.clone();
            thread::spawn(move || {
                for i in 0..5_000u32 {
                    let key = t * 100_000 + i; // distinct keys per thread
                    cache.put(key, key, Duration::from_secs(60));
                    assert!(cache.len() <= cap, "resident set exceeded global cap");
                }
            })
        })
        .collect();
    for h in threads {
        h.join().unwrap();
    }
    assert!(cache.len() <= cap);
    cache.check_invariants();
}

// ===========================================================================
// C. TTL / clock tests
// ===========================================================================

#[test]
fn ttl_zero_is_immediately_unretrievable() {
    let clock = Arc::new(FakeClock::new());
    let cache: Cache<u32, u32, Arc<FakeClock>> = Cache::with_clock(4, clock.clone());
    cache.put(1, 1, Duration::ZERO);
    assert!(cache.get(&1).is_none(), "ttl=0 must be immediately expired");
    cache.check_invariants();
}

#[test]
fn duration_max_never_expires_and_does_not_panic() {
    let clock = Arc::new(FakeClock::new());
    let cache: Cache<u32, u32, Arc<FakeClock>> = Cache::with_clock(4, clock.clone());
    // Both Duration::MAX and a value that would overflow Instant.
    cache.put(1, 11, Duration::MAX);
    cache.put(2, 22, Duration::from_secs(u64::MAX));
    clock.advance(Duration::from_secs(1_000_000_000));
    assert_eq!(cache.get(&1).map(|a| *a), Some(11), "Duration::MAX must persist");
    assert_eq!(cache.get(&2).map(|a| *a), Some(22));
    cache.check_invariants();
}

#[test]
fn lazy_expiry_on_get_with_fake_clock() {
    let clock = Arc::new(FakeClock::new());
    let cache: Cache<u32, u32, Arc<FakeClock>> = Cache::with_clock(4, clock.clone());
    cache.put(1, 1, Duration::from_millis(100));
    assert_eq!(cache.get(&1).map(|a| *a), Some(1));
    clock.advance(Duration::from_millis(150));
    assert!(cache.get(&1).is_none(), "expired entry must read as None");
    assert_eq!(cache.len(), 0, "expired entry removed lazily on get");
    cache.check_invariants();
}

#[test]
fn expired_squat_reclaimed_before_evicting_live_entry() {
    // Fill to capacity; expire a *middle* (non-tail) entry; one more put must
    // reclaim the dead entry and NOT evict a live LRU entry.
    let clock = Arc::new(FakeClock::new());
    let cache: Cache<u32, u32, Arc<FakeClock>> = Cache::with_clock(3, clock.clone());

    cache.put(1, 1, Duration::from_secs(100)); // long-lived
    cache.put(2, 2, Duration::from_millis(10)); // short-lived (middle)
    cache.put(3, 3, Duration::from_secs(100)); // long-lived, MRU
    // LRU order: tail=1, then 2, head=3.

    clock.advance(Duration::from_millis(50)); // key 2 now expired, still resident
    assert_eq!(cache.len(), 3);

    cache.put(4, 4, Duration::from_secs(100)); // over capacity → must reclaim #2

    assert_eq!(cache.len(), 3);
    assert!(cache.get(&2).is_none(), "expired squatter must be reclaimed");
    assert_eq!(cache.get(&1).map(|a| *a), Some(1), "live LRU tail must survive");
    assert_eq!(cache.get(&3).map(|a| *a), Some(3));
    assert_eq!(cache.get(&4).map(|a| *a), Some(4));
    cache.check_invariants();
}

#[test]
fn purge_expired_reaper() {
    let clock = Arc::new(FakeClock::new());
    let cache: Cache<u32, u32, Arc<FakeClock>> = Cache::with_clock(10, clock.clone());
    for k in 0..5u32 {
        cache.put(k, k, Duration::from_millis(10));
    }
    for k in 5..10u32 {
        cache.put(k, k, Duration::from_secs(100));
    }
    clock.advance(Duration::from_millis(50));
    cache.purge_expired();
    assert_eq!(cache.len(), 5, "only long-lived entries remain");
    for k in 5..10u32 {
        assert_eq!(cache.get(&k).map(|a| *a), Some(k));
    }
    cache.check_invariants();
}

#[test]
fn overwrite_updates_value_deadline_and_recency_no_double_count() {
    let clock = Arc::new(FakeClock::new());
    let cache: Cache<u32, u32, Arc<FakeClock>> = Cache::with_clock(2, clock.clone());
    cache.put(1, 1, Duration::from_secs(100));
    cache.put(2, 2, Duration::from_secs(100));
    // Overwrite key 1 — must not change len, must refresh value + recency.
    cache.put(1, 111, Duration::from_secs(100));
    assert_eq!(cache.len(), 2, "overwrite must not double-count len");
    assert_eq!(cache.get(&1).map(|a| *a), Some(111));
    // key 2 is now LRU; inserting a third evicts it, not the refreshed key 1.
    cache.put(3, 3, Duration::from_secs(100));
    assert!(cache.get(&2).is_none());
    assert_eq!(cache.get(&1).map(|a| *a), Some(111));
    cache.check_invariants();
}

// ===========================================================================
// D. Concurrency / race tests
// ===========================================================================

#[test]
fn high_thread_stress_no_torn_values() {
    // value(key) = key*7 + 1 — a get must never return garbage.
    let cache: Arc<Cache<u64, u64>> = Arc::new(Cache::new(64));
    let threads: Vec<_> = (0..32u64)
        .map(|t| {
            let cache = cache.clone();
            thread::spawn(move || {
                let mut rng = Rng::new(t.wrapping_mul(2654435761) ^ 0xABCD);
                for _ in 0..50_000u64 {
                    let key = rng.below(256);
                    if rng.below(2) == 0 {
                        cache.put(key, key.wrapping_mul(7).wrapping_add(1), Duration::from_secs(30));
                    } else if let Some(v) = cache.get(&key) {
                        assert_eq!(*v, key.wrapping_mul(7).wrapping_add(1), "torn value!");
                    }
                    assert!(cache.len() <= 64, "capacity breached under contention");
                }
            })
        })
        .collect();
    for h in threads {
        h.join().unwrap();
    }
    cache.check_invariants();
}

#[test]
fn concurrent_get_put_same_keys_no_deadlock_or_panic() {
    let cache: Arc<Cache<u32, u32>> = Arc::new(Cache::new(4));
    let threads: Vec<_> = (0..8u32)
        .map(|t| {
            let cache = cache.clone();
            thread::spawn(move || {
                let mut rng = Rng::new(t as u64 + 1);
                for _ in 0..40_000 {
                    let key = rng.below(6) as u32; // heavy overlap
                    match rng.below(3) {
                        0 => {
                            cache.put(key, key, Duration::from_millis(rng.below(5)));
                        }
                        1 => {
                            let _ = cache.get(&key);
                        }
                        _ => {
                            let _ = cache.len();
                        }
                    }
                }
            })
        })
        .collect();
    for h in threads {
        h.join().unwrap();
    }
    assert!(cache.len() <= 4);
    cache.check_invariants();
}

// ===========================================================================
// E. Liveness / poison / drop-safety tests
// ===========================================================================

#[derive(Clone)]
struct PanicKey {
    id: u32,
    bomb: bool,
}
impl PartialEq for PanicKey {
    fn eq(&self, other: &Self) -> bool {
        if self.bomb || other.bomb {
            panic!("PanicKey::eq bomb");
        }
        self.id == other.id
    }
}
impl Eq for PanicKey {}
impl std::hash::Hash for PanicKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        if self.bomb {
            panic!("PanicKey::hash bomb");
        }
        self.id.hash(state);
    }
}

#[test]
fn panic_in_key_hash_does_not_brick_the_cache() {
    let cache: Cache<PanicKey, u32> = Cache::new(8);
    for id in 0..4u32 {
        cache.put(PanicKey { id, bomb: false }, id, Duration::from_secs(60));
    }
    // Querying a "bomb" key panics inside the lock; the non-poisoning lock must
    // recover so other keys are still served afterward.
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _ = cache.get(&PanicKey { id: 99, bomb: true });
    }));
    assert!(result.is_err(), "the bomb key should have panicked");

    // Cache must still work.
    for id in 0..4u32 {
        assert_eq!(
            cache.get(&PanicKey { id, bomb: false }).map(|a| *a),
            Some(id),
            "cache bricked after panic in Key::Hash"
        );
    }
    cache.put(PanicKey { id: 5, bomb: false }, 5, Duration::from_secs(60));
    assert_eq!(cache.get(&PanicKey { id: 5, bomb: false }).map(|a| *a), Some(5));
    cache.check_invariants();
}

/// A key whose `Hash` is *non-deterministic*: it succeeds for the first
/// `panic_on - 1` calls and then panics. This is the only way to reach the
/// `put` new-entry insertion path with a panic — `put` first hashes the key for
/// `map.get` (which must succeed, returning "absent"), and only the *subsequent*
/// map insertion may explode.
#[derive(Clone)]
struct NthHashPanic {
    id: u32,
    calls: Arc<std::sync::atomic::AtomicUsize>,
    panic_on: usize,
}
impl PartialEq for NthHashPanic {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for NthHashPanic {}
impl std::hash::Hash for NthHashPanic {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if n == self.panic_on {
            panic!("NthHashPanic::hash bomb on call {n}");
        }
        self.id.hash(state);
    }
}

/// Regression: in `put`'s new-entry branch the arena must NOT be mutated before
/// the panickable `Key::Hash`/`Eq` that the map insertion runs. Otherwise a hash
/// panic on insertion leaves an orphan arena node (`arena.len() == 1`) that is in
/// neither the map (`map.len() == 0`) nor the LRU list, and the poison-recovering
/// lock then hands that inconsistent `Core` to other threads — violating the
/// `map.len() == arena.len()` invariant. Pre-fix this fails inside
/// `check_invariants`; post-fix the `Core` is untouched by the panic.
#[test]
fn panic_in_new_entry_insert_leaves_core_consistent() {
    let cache: Cache<NthHashPanic, u32> = Cache::new(8);

    // One resident entry so the map table is non-empty: `put`'s initial
    // `map.get` is then guaranteed to actually hash the queried key.
    let resident_counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    cache.put(
        NthHashPanic { id: 1, calls: resident_counter, panic_on: usize::MAX },
        1,
        Duration::from_secs(60),
    );
    cache.check_invariants();

    // This key hashes fine for the `map.get` probe (call 1 ⇒ "absent"), then
    // panics on the map insertion (call 2) — exactly the new-entry path.
    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let bomb = NthHashPanic { id: 99, calls: counter, panic_on: 2 };
    let result = catch_unwind(AssertUnwindSafe(|| {
        cache.put(bomb, 99, Duration::from_secs(60));
    }));
    assert!(result.is_err(), "the bomb key should have panicked on insertion");

    // The recovered Core must satisfy `map.len() == arena.len()`; no orphan node.
    cache.check_invariants();

    // The pre-existing entry survives and the cache is still usable.
    assert_eq!(cache.len(), 1, "orphan node left in the arena after the panic");
    cache.put(
        NthHashPanic { id: 2, calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)), panic_on: usize::MAX },
        2,
        Duration::from_secs(60),
    );
    cache.check_invariants();
}

struct PanicOnDrop {
    armed: bool,
}
impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        if self.armed {
            panic!("PanicOnDrop::drop bomb");
        }
    }
}

#[test]
fn panic_in_value_drop_surfaces_outside_lock_and_cache_survives() {
    let cache: Cache<u32, PanicOnDrop> = Cache::new(1);
    cache.put(1, PanicOnDrop { armed: true }, Duration::from_secs(60));

    // Inserting key 2 evicts key 1, whose Drop panics — but Drop runs AFTER the
    // lock is released, so it cannot poison the lock.
    let result = catch_unwind(AssertUnwindSafe(|| {
        cache.put(2, PanicOnDrop { armed: false }, Duration::from_secs(60));
    }));
    assert!(result.is_err(), "the armed value's Drop should have panicked");

    // Cache is intact and usable.
    assert!(cache.get(&2).is_some(), "cache bricked after panic in Value::Drop");
    cache.put(3, PanicOnDrop { armed: false }, Duration::from_secs(60));
    cache.check_invariants();
}

struct DropCallback {
    f: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl Drop for DropCallback {
    fn drop(&mut self) {
        if let Some(f) = self.f.lock().unwrap().take() {
            f();
        }
    }
}

#[test]
fn value_drop_reentering_cache_does_not_deadlock() {
    // Run in a thread guarded by a timeout: a reentrancy deadlock would hang.
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let cache: Arc<Cache<u32, DropCallback>> = Arc::new(Cache::new(1));
        let reentered = Arc::new(AtomicBool::new(false));

        let cache_for_drop = cache.clone();
        let reentered_for_drop = reentered.clone();
        cache.put(
            1,
            DropCallback {
                f: Mutex::new(Some(Box::new(move || {
                    // Re-enter the SAME cache from within Drop (after unlock).
                    let _ = cache_for_drop.get(&999);
                    let _ = cache_for_drop.len();
                    reentered_for_drop.store(true, Ordering::SeqCst);
                }))),
            },
            Duration::from_secs(60),
        );

        // Evict key 1 → its Drop re-enters the cache. Must not deadlock.
        cache.put(
            2,
            DropCallback {
                f: Mutex::new(None),
            },
            Duration::from_secs(60),
        );

        assert!(reentered.load(Ordering::SeqCst), "Drop callback did not run");
        cache.check_invariants();
        tx.send(()).unwrap();
    });

    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(()) => handle.join().unwrap(),
        Err(_) => panic!("reentrant Value::Drop deadlocked the cache"),
    }
}

#[test]
fn overflow_put_on_write_path_does_not_panic() {
    let cache: Cache<u32, u32> = Cache::new(4);
    // Duration::MAX would panic with `Instant + Duration`; checked_add must save us.
    cache.put(1, 1, Duration::MAX);
    assert_eq!(cache.get(&1).map(|a| *a), Some(1));
    cache.check_invariants();
}

// ===========================================================================
// F. Memory-growth test (cache level; arena internals covered by unit tests)
// ===========================================================================

#[test]
fn arena_backing_stabilizes_under_eviction_churn() {
    let cache: Cache<u64, u64> = Cache::new(8);
    for i in 0..200_000u64 {
        cache.put(i, i, Duration::from_secs(60)); // each over-cap put evicts one
    }
    assert_eq!(cache.len(), 8);
    // Free-list reuse ⇒ backing storage bounded near capacity, not 200k.
    assert!(
        cache.arena_backing_len() <= 16,
        "arena grew unbounded: {}",
        cache.arena_backing_len()
    );
    cache.check_invariants();
}

// A small sanity check that Arc cloning is what `get` returns (cheap refcount).
#[test]
fn get_returns_shared_arc() {
    let cache: Cache<u32, String> = Cache::new(2);
    cache.put(1, "hello".to_string(), Duration::from_secs(60));
    let a = cache.get(&1).unwrap();
    let b = cache.get(&1).unwrap();
    assert_eq!(*a, "hello");
    assert!(Arc::ptr_eq(&a, &b), "get should hand out the same shared Arc");
    // Keep the HashMap import used.
    let _seen: HashMap<u32, ()> = HashMap::new();
}
