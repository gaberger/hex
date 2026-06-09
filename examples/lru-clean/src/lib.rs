//! Thread-safe LRU cache with per-entry TTL — built by the full hex harness.
//!
//! Implements the build spec in the challenge: exact global capacity, exact
//! global LRU, per-entry TTL, safe under many concurrent threads. One lock over
//! one coherent structure (HashMap + generational arena + intrusive LRU list +
//! BTreeMap expiry index). No `unsafe`. Liveness and edge-cases designed out:
//!
//! - `get` returns `Arc<V>` (refcount bump under the lock, never a payload copy).
//! - Non-poisoning lock: a poison-recovering wrapper around `std::sync::Mutex`,
//!   so a panic in `Key::Hash`/`Eq` cannot permanently brick the cache.
//! - User values are dropped *outside* the lock (no poison / no reentrant
//!   deadlock from `Value::Drop`).
//! - Deadlines use `Instant` (monotonic) and `checked_add` (no overflow panic,
//!   `Duration::MAX` ⇒ "never expires").
//! - A BTreeMap expiry index lets eviction reclaim an already-dead entry before
//!   evicting a live LRU entry (kills the "dead squats / evict-live" bug).

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Clock abstraction (injectable for deterministic TTL testing)
// ---------------------------------------------------------------------------

/// Monotonic time source. The cache reads `now()` *before* taking the lock.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// Real monotonic clock backed by `Instant::now()`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Controllable fake clock for tests. Monotonic; only moves when `advance` is
/// called. Backed by a real `Instant` base plus an atomic nanosecond offset so
/// many threads can read it concurrently.
#[derive(Debug)]
pub struct FakeClock {
    base: Instant,
    offset_nanos: AtomicU64,
}

impl FakeClock {
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            offset_nanos: AtomicU64::new(0),
        }
    }

    /// Advance the fake clock forward by `d`. Monotonic — never moves backward.
    pub fn advance(&self, d: Duration) {
        let add = d.as_nanos().min(u64::MAX as u128) as u64;
        self.offset_nanos.fetch_add(add, Ordering::SeqCst);
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        let off = self.offset_nanos.load(Ordering::SeqCst);
        self.base
            .checked_add(Duration::from_nanos(off))
            .unwrap_or(self.base)
    }
}

// A blanket impl so `Arc<C: Clock>` is itself a Clock — lets tests share one
// clock between the cache and the reference model.
impl<C: Clock + ?Sized> Clock for Arc<C> {
    fn now(&self) -> Instant {
        (**self).now()
    }
}

// ---------------------------------------------------------------------------
// Poison-recovering lock wrapper
// ---------------------------------------------------------------------------

/// A `std::sync::Mutex` that recovers from poisoning instead of propagating it.
/// A panic while holding the lock no longer bricks the cache for every other
/// thread. The spec argues correctness: panickable user code (`Key::Hash`/`Eq`)
/// runs at the *start* of an operation before any mutation, so a recovered
/// `Core` is consistent. User `Value::Drop`/`Clone` never run under this lock.
struct Lock<T> {
    inner: Mutex<T>,
}

impl<T> Lock<T> {
    fn new(value: T) -> Self {
        Self {
            inner: Mutex::new(value),
        }
    }

    fn lock(&self) -> MutexGuard<'_, T> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        }
    }
}

// ---------------------------------------------------------------------------
// Generational arena
// ---------------------------------------------------------------------------

/// Handle into the arena. Carries a generation so a stale handle to a recycled
/// slot is rejected rather than silently aliasing (closes the ABA hole).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotId {
    index: u32,
    generation: u32,
}

enum Slot<T> {
    Occupied { generation: u32, value: T },
    Free { generation: u32, next_free: Option<u32> },
}

impl<T> Slot<T> {
    #[cfg(test)]
    fn generation(&self) -> u32 {
        match self {
            Slot::Occupied { generation, .. } => *generation,
            Slot::Free { generation, .. } => *generation,
        }
    }
}

struct Arena<T> {
    slots: Vec<Slot<T>>,
    free_head: Option<u32>,
    len: usize,
}

impl<T> Arena<T> {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_head: None,
            len: 0,
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn insert(&mut self, value: T) -> SlotId {
        self.len += 1;
        match self.free_head {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                let (generation, next_free) = match slot {
                    Slot::Free {
                        generation,
                        next_free,
                    } => (*generation, *next_free),
                    Slot::Occupied { .. } => unreachable!("free_head points at occupied slot"),
                };
                self.free_head = next_free;
                *slot = Slot::Occupied { generation, value };
                SlotId { index, generation }
            }
            None => {
                let index = self.slots.len() as u32;
                self.slots.push(Slot::Occupied {
                    generation: 0,
                    value,
                });
                SlotId {
                    index,
                    generation: 0,
                }
            }
        }
    }

    fn get(&self, id: SlotId) -> Option<&T> {
        match self.slots.get(id.index as usize) {
            Some(Slot::Occupied { generation, value }) if *generation == id.generation => {
                Some(value)
            }
            _ => None,
        }
    }

    fn get_mut(&mut self, id: SlotId) -> Option<&mut T> {
        match self.slots.get_mut(id.index as usize) {
            Some(Slot::Occupied { generation, value }) if *generation == id.generation => {
                Some(value)
            }
            _ => None,
        }
    }

    fn remove(&mut self, id: SlotId) -> Option<T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        match slot {
            Slot::Occupied { generation, .. } if *generation == id.generation => {
                // Bump generation so any stale handle is rejected from now on.
                let new_gen = generation.wrapping_add(1);
                let old = std::mem::replace(
                    slot,
                    Slot::Free {
                        generation: new_gen,
                        next_free: self.free_head,
                    },
                );
                self.free_head = Some(id.index);
                self.len -= 1;
                match old {
                    Slot::Occupied { value, .. } => Some(value),
                    Slot::Free { .. } => unreachable!(),
                }
            }
            _ => None,
        }
    }

    /// Total backing capacity (for the memory-growth test).
    fn backing_len(&self) -> usize {
        self.slots.len()
    }

    /// True if `id` no longer refers to a live slot (generation mismatch / freed).
    #[cfg(test)]
    fn is_stale(&self, id: SlotId) -> bool {
        match self.slots.get(id.index as usize) {
            Some(slot) => slot.generation() != id.generation
                || matches!(slot, Slot::Free { .. }),
            None => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Core (everything behind the one lock)
// ---------------------------------------------------------------------------

struct Node<K, V> {
    key: K,
    value: Arc<V>,
    /// `None` = never expires.
    deadline: Option<Instant>,
    prev: Option<SlotId>,
    next: Option<SlotId>,
}

struct Core<K, V> {
    capacity: usize,
    map: HashMap<K, SlotId>,
    arena: Arena<Node<K, V>>,
    head: Option<SlotId>, // most-recently-used
    tail: Option<SlotId>, // least-recently-used (next eviction victim)
    expiry: BTreeMap<(Instant, SlotId), ()>,
}

impl<K, V> Core<K, V>
where
    K: Hash + Eq + Clone,
{
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            map: HashMap::new(),
            arena: Arena::new(),
            head: None,
            tail: None,
            expiry: BTreeMap::new(),
        }
    }

    fn unlink(&mut self, id: SlotId) {
        let (prev, next) = {
            let node = self.arena.get(id).expect("unlink: live slot");
            (node.prev, node.next)
        };
        match prev {
            Some(p) => self.arena.get_mut(p).expect("unlink prev").next = next,
            None => self.head = next,
        }
        match next {
            Some(n) => self.arena.get_mut(n).expect("unlink next").prev = prev,
            None => self.tail = prev,
        }
        let node = self.arena.get_mut(id).expect("unlink self");
        node.prev = None;
        node.next = None;
    }

    fn push_front(&mut self, id: SlotId) {
        let old_head = self.head;
        {
            let node = self.arena.get_mut(id).expect("push_front self");
            node.prev = None;
            node.next = old_head;
        }
        match old_head {
            Some(h) => self.arena.get_mut(h).expect("push_front old head").prev = Some(id),
            None => self.tail = Some(id),
        }
        self.head = Some(id);
    }

    fn move_to_front(&mut self, id: SlotId) {
        if self.head == Some(id) {
            return;
        }
        self.unlink(id);
        self.push_front(id);
    }

    /// Remove a slot from every structure and return its value `Arc` so the
    /// caller can drop it outside the lock.
    fn remove_slot(&mut self, id: SlotId) -> Arc<V> {
        self.unlink(id);
        let node = self.arena.remove(id).expect("remove_slot: live slot");
        self.map.remove(&node.key);
        if let Some(d) = node.deadline {
            self.expiry.remove(&(d, id));
        }
        node.value
    }

    /// Evict exactly one entry under the capacity policy: prefer an
    /// already-expired entry (cheap O(log n) front-of-index peek), otherwise
    /// the true global LRU tail.
    fn evict_one(&mut self, now: Instant, to_drop: &mut Vec<Arc<V>>) {
        if let Some((&(deadline, id), _)) = self.expiry.iter().next() {
            if deadline <= now {
                to_drop.push(self.remove_slot(id));
                return;
            }
        }
        if let Some(tail) = self.tail {
            to_drop.push(self.remove_slot(tail));
        }
    }

    fn check_invariants(&self) {
        let live = self.arena.len();
        assert_eq!(self.map.len(), live, "map.len != arena live count");
        assert!(self.map.len() <= self.capacity, "capacity exceeded");

        // Walk the list head -> tail; verify bijection with the map and link
        // consistency; bound the walk so a cycle can't loop forever.
        let mut count = 0usize;
        let mut cur = self.head;
        let mut prev: Option<SlotId> = None;
        let mut expiry_expected = 0usize;
        while let Some(id) = cur {
            assert!(count <= live, "LRU list longer than arena (cycle?)");
            let node = self.arena.get(id).expect("list slot live");
            assert_eq!(node.prev, prev, "prev link inconsistent");
            let mapped = self.map.get(&node.key).copied();
            assert_eq!(mapped, Some(id), "map<->list disagree for key");
            if let Some(d) = node.deadline {
                assert!(
                    self.expiry.contains_key(&(d, id)),
                    "live deadline missing from expiry index"
                );
                expiry_expected += 1;
            }
            prev = Some(id);
            cur = node.next;
            count += 1;
        }
        assert_eq!(count, live, "list length != arena live count");
        assert_eq!(self.tail, prev, "tail does not match end of list");

        // Expiry index contains exactly the live timed slots, and no others.
        assert_eq!(
            self.expiry.len(),
            expiry_expected,
            "expiry index has stale / extra entries"
        );
        for &(d, id) in self.expiry.keys() {
            let node = self.arena.get(id).expect("expiry slot live");
            assert_eq!(node.deadline, Some(d), "expiry deadline mismatch");
        }
    }
}

// ---------------------------------------------------------------------------
// Public cache
// ---------------------------------------------------------------------------

/// Thread-safe, bounded LRU cache with per-entry TTL.
///
/// Share via `Arc<Cache<K, V>>` and clone the `Arc` per thread. `Send + Sync`.
pub struct Cache<K, V, C: Clock = SystemClock> {
    inner: Lock<Core<K, V>>,
    clock: C,
}

impl<K, V> Cache<K, V, SystemClock>
where
    K: Hash + Eq + Clone + Send,
    V: Send + Sync,
{
    /// Create a cache with the given fixed capacity.
    ///
    /// `capacity == 0` is legal and means "cache nothing": every `put` inserts
    /// then immediately evicts, so `len()` is always 0.
    pub fn new(capacity: usize) -> Self {
        Self::with_clock(capacity, SystemClock)
    }
}

impl<K, V, C> Cache<K, V, C>
where
    K: Hash + Eq + Clone + Send,
    V: Send + Sync,
    C: Clock,
{
    /// Create a cache with an injected clock (used by deterministic tests).
    pub fn with_clock(capacity: usize, clock: C) -> Self {
        Self {
            inner: Lock::new(Core::new(capacity)),
            clock,
        }
    }

    /// Insert or overwrite `key` with `value`, expiring after `ttl`.
    ///
    /// `put` counts as a use (moves the entry to MRU). `ttl == 0` inserts an
    /// immediately-expired entry; a `ttl` that overflows `Instant` (e.g.
    /// `Duration::MAX`) is treated as "never expires".
    pub fn put(&self, key: K, value: V, ttl: Duration) {
        let now = self.clock.now();
        // Dropped after the lock is released.
        let mut to_drop: Vec<Arc<V>> = Vec::new();
        {
            let mut core = self.inner.lock();
            let deadline = now.checked_add(ttl); // None on overflow ⇒ never expires
            let arc = Arc::new(value);

            if let Some(id) = core.map.get(&key).copied() {
                // Overwrite path — no count change (len reads map.len()).
                let (old_deadline, old_value) = {
                    let node = core.arena.get_mut(id).expect("overwrite live slot");
                    let old_deadline = node.deadline;
                    let old_value = std::mem::replace(&mut node.value, arc);
                    node.deadline = deadline;
                    (old_deadline, old_value)
                };
                if let Some(od) = old_deadline {
                    core.expiry.remove(&(od, id));
                }
                if let Some(d) = deadline {
                    core.expiry.insert((d, id), ());
                }
                to_drop.push(old_value);
                core.move_to_front(id);
            } else {
                // New entry. Reserve the map slot *first* so the user-panickable
                // `Key::Hash`/`Eq` (the hash probe) runs before any mutation of
                // the arena/list. If it panics here the Core is untouched, so the
                // poison-recovering Lock can hand it to other threads with the
                // `map.len() == arena.len()` invariant intact. (Disjoint borrows
                // of `map` and `arena` need a single `&mut Core`, not the guard.)
                let node = Node {
                    key: key.clone(),
                    value: arc,
                    deadline,
                    prev: None,
                    next: None,
                };
                let core: &mut Core<K, V> = &mut core;
                let slot = match core.map.entry(key) {
                    std::collections::hash_map::Entry::Occupied(_) => {
                        unreachable!("map.get returned None for this key above")
                    }
                    std::collections::hash_map::Entry::Vacant(slot) => slot,
                };
                let id = core.arena.insert(node);
                slot.insert(id);
                core.push_front(id);
                if let Some(d) = deadline {
                    core.expiry.insert((d, id), ());
                }
                // Enforce capacity.
                while core.map.len() > core.capacity {
                    core.evict_one(now, &mut to_drop);
                }
            }
        }
        drop(to_drop);
    }

    /// Fetch `key`. Returns `None` if absent or expired; an expired entry is
    /// removed lazily. A hit counts as a use (moves the entry to MRU).
    pub fn get(&self, key: &K) -> Option<Arc<V>> {
        let now = self.clock.now();
        let mut to_drop: Vec<Arc<V>> = Vec::new();
        let result;
        {
            let mut core = self.inner.lock();
            match core.map.get(key).copied() {
                None => result = None,
                Some(id) => {
                    let deadline = core.arena.get(id).expect("get live slot").deadline;
                    match deadline {
                        Some(d) if d <= now => {
                            to_drop.push(core.remove_slot(id));
                            result = None;
                        }
                        _ => {
                            core.move_to_front(id);
                            let value = core.arena.get(id).expect("get live slot").value.clone();
                            result = Some(value);
                        }
                    }
                }
            }
        }
        drop(to_drop);
        result
    }

    /// Number of physically-resident entries — an upper bound on live entries
    /// (may include expired-but-not-yet-reaped entries). Never exceeds capacity.
    pub fn len(&self) -> usize {
        self.inner.lock().map.len()
    }

    /// True when no entries are resident.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Configured capacity.
    pub fn capacity(&self) -> usize {
        self.inner.lock().capacity
    }

    /// Active reaper (documented extension, off the hot path): remove every
    /// entry whose deadline has passed. O(k log n) for k reaped.
    pub fn purge_expired(&self) {
        let now = self.clock.now();
        let mut to_drop: Vec<Arc<V>> = Vec::new();
        {
            let mut core = self.inner.lock();
            loop {
                let next = core.expiry.iter().next().map(|(&(d, id), _)| (d, id));
                match next {
                    Some((d, id)) if d <= now => {
                        to_drop.push(core.remove_slot(id));
                    }
                    _ => break,
                }
            }
        }
        drop(to_drop);
    }

    /// Assert every structural invariant (§6). Test/debug helper.
    pub fn check_invariants(&self) {
        self.inner.lock().check_invariants();
    }

    /// Total backing storage of the arena (memory-growth test helper).
    pub fn arena_backing_len(&self) -> usize {
        self.inner.lock().arena.backing_len()
    }
}

// ---------------------------------------------------------------------------
// In-crate unit tests for the generational arena (category F internals)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod arena_tests {
    use super::*;

    #[test]
    fn reuses_freed_slots_and_bumps_generation() {
        let mut arena: Arena<u32> = Arena::new();
        let a = arena.insert(10);
        assert_eq!(arena.len(), 1);
        let removed = arena.remove(a);
        assert_eq!(removed, Some(10));
        assert_eq!(arena.len(), 0);

        // Re-insert reuses the same index but with a higher generation.
        let b = arena.insert(20);
        assert_eq!(a.index, b.index, "freed index should be reused");
        assert_ne!(a.generation, b.generation, "generation must bump on reuse");
    }

    #[test]
    fn stale_handle_is_rejected_not_aliased() {
        let mut arena: Arena<u32> = Arena::new();
        let stale = arena.insert(1);
        arena.remove(stale);
        let fresh = arena.insert(2); // recycles stale's index

        // The stale handle must not alias the recycled live entry.
        assert!(arena.is_stale(stale), "stale handle must report stale");
        assert_eq!(arena.get(stale), None, "stale handle must not read");
        assert_eq!(arena.remove(stale), None, "stale handle must not remove");
        // The fresh handle still works.
        assert_eq!(arena.get(fresh), Some(&2));
        assert_eq!(arena.len(), 1);
    }

    #[test]
    fn backing_storage_stabilizes_under_churn() {
        let mut arena: Arena<u32> = Arena::new();
        // Keep at most 4 live; churn many cycles. Backing length must stay
        // bounded by the high-water mark, not grow unboundedly.
        let mut live: Vec<SlotId> = Vec::new();
        for i in 0..10_000u32 {
            live.push(arena.insert(i));
            if live.len() > 4 {
                let id = live.remove(0);
                arena.remove(id);
            }
        }
        assert!(arena.len() <= 5);
        assert!(
            arena.backing_len() <= 5,
            "backing storage grew unbounded: {}",
            arena.backing_len()
        );
    }
}
