//! Thread-safe token-bucket rate limiter — built by the hex harness.
//!
//! Design: a single [`Mutex`] over **fixed-point integer** state. Token
//! accounting lives entirely in the scaled domain (sub-tokens at `SCALE =
//! 2^32`), so there is exactly one linearization point per call and no `f64`
//! anywhere in the runtime accounting path. `f64` appears once, in
//! [`RateLimiter::new`], to convert the rate — rounded toward under-issue.
//!
//! See the build spec for the full rationale; the short version:
//! integer accounting structurally eliminates `f64` over-issue above 2^53 and
//! the floor/ceil burst failure, and `saturating_duration_since` makes a
//! merely-monotonic clock robust without ever consulting a wall clock.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Fixed-point scale: one whole token == `2^32` sub-tokens.
const SCALE_BITS: u32 = 32;
/// `2^32` as `f64`, used only for the one-time rate conversion in `new()`.
const SCALE_F: f64 = 4_294_967_296.0; // 2f64.powi(32)
const NANOS_PER_SEC: u128 = 1_000_000_000;

/// Error returned by [`RateLimiter::try_new`] for an invalid rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateError {
    /// `refill_per_sec` was NaN, infinite, or `<= 0`.
    NotPositiveFinite,
    /// `refill_per_sec` is so small it rounds to zero sub-tokens/sec at our scale.
    TooSmall,
}

impl std::fmt::Display for RateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RateError::NotPositiveFinite => {
                write!(f, "refill_per_sec must be a finite number > 0")
            }
            RateError::TooSmall => write!(
                f,
                "refill_per_sec is too small: it rounds to zero sub-tokens/sec at scale 2^32"
            ),
        }
    }
}

impl std::error::Error for RateError {}

/// Time source seam. Production uses [`MonotonicClock`]; tests use a manual one.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// Production clock — wraps `Instant::now()`.
#[derive(Debug, Default, Clone, Copy)]
pub struct MonotonicClock;

impl Clock for MonotonicClock {
    #[inline]
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Mutable bucket state — always held behind the one mutex.
struct State {
    /// Current balance in sub-tokens, always kept in `[0, capacity_scaled]`.
    tokens_scaled: u128,
    /// Monotonic anchor at which `tokens_scaled` was last brought current.
    last: Instant,
}

/// A thread-safe token-bucket rate limiter.
///
/// Share it across threads via `Arc<RateLimiter>`. The type is generic over a
/// [`Clock`] only to allow deterministic tests; the public [`new`](RateLimiter::new)
/// constructor pins the production [`MonotonicClock`].
pub struct RateLimiter<C: Clock = MonotonicClock> {
    capacity_scaled: u128,
    rate_scaled_per_sec: u128,
    clock: C,
    state: Mutex<State>,
}

impl RateLimiter<MonotonicClock> {
    /// Create a full bucket of `capacity` whole tokens refilling at
    /// `refill_per_sec` tokens/second.
    ///
    /// # Panics
    /// Panics if `refill_per_sec` is NaN, infinite, or `<= 0`, or if the rate
    /// is so small it rounds to zero sub-tokens/sec at our `2^32` scale.
    /// `capacity == 0` is legal and yields a bucket that rejects every `n >= 1`.
    ///
    /// Use [`try_new`](RateLimiter::try_new) for the non-panicking form.
    pub fn new(capacity: u64, refill_per_sec: f64) -> Self {
        Self::try_with_clock(capacity, refill_per_sec, MonotonicClock)
            .unwrap_or_else(|e| panic!("RateLimiter::new: {e}"))
    }

    /// Non-panicking constructor — validates the rate at the boundary.
    pub fn try_new(capacity: u64, refill_per_sec: f64) -> Result<Self, RateError> {
        Self::try_with_clock(capacity, refill_per_sec, MonotonicClock)
    }
}

impl<C: Clock> RateLimiter<C> {
    /// Construct with an explicit clock (test seam).
    fn try_with_clock(capacity: u64, refill_per_sec: f64, clock: C) -> Result<Self, RateError> {
        if refill_per_sec.is_nan() || refill_per_sec.is_infinite() || refill_per_sec <= 0.0 {
            return Err(RateError::NotPositiveFinite);
        }
        // Floor toward under-issue: we never refill faster than specified.
        let rate_scaled_per_sec = (refill_per_sec * SCALE_F).floor() as u128;
        if rate_scaled_per_sec == 0 {
            return Err(RateError::TooSmall);
        }
        let capacity_scaled = (capacity as u128) << SCALE_BITS;
        let now = clock.now();
        Ok(RateLimiter {
            capacity_scaled,
            rate_scaled_per_sec,
            clock,
            state: Mutex::new(State {
                tokens_scaled: capacity_scaled,
                last: now,
            }),
        })
    }

    /// The single shared refill helper — used by both `try_acquire` and
    /// `available` so they can never diverge.
    fn refill_locked(&self, state: &mut State, now: Instant) {
        // saturating: a non-advancing or apparently-backward monotonic clock
        // is a no-op (elapsed 0), never a panic or negative credit. No wall
        // clock is ever consulted.
        let elapsed_ns = now.saturating_duration_since(state.last).as_nanos();
        if elapsed_ns == 0 {
            return;
        }
        // rate (<=~2^62) * elapsed_ns (<=~2^97) can exceed u128 only for
        // absurd durations; saturating_mul then yields a huge `accrued` that
        // simply lands in the capped branch — never an over-issue.
        let accrued = self.rate_scaled_per_sec.saturating_mul(elapsed_ns) / NANOS_PER_SEC;

        if state.tokens_scaled.saturating_add(accrued) >= self.capacity_scaled {
            // Cap: idle overflow time is forfeited, never banked. This is what
            // stops a long idle from later granting more than `capacity`.
            state.tokens_scaled = self.capacity_scaled;
            state.last = now;
        } else {
            state.tokens_scaled += accrued;
            // Advance the anchor by ONLY the time the credited tokens consumed.
            // When elapsed is tiny and `accrued` floors to 0, `consumed_ns` is
            // 0 and `last` does not move — fractional time accumulates until it
            // credits a whole sub-token (no high-frequency-poll starvation).
            let consumed_ns = accrued * NANOS_PER_SEC / self.rate_scaled_per_sec;
            let secs = (consumed_ns / NANOS_PER_SEC) as u64;
            let nanos = (consumed_ns % NANOS_PER_SEC) as u32;
            state.last += Duration::new(secs, nanos);
        }
    }

    /// Atomically refill by elapsed monotonic time (capped at capacity), then
    /// consume `n` iff at least `n` whole tokens are available. Returns `true`
    /// on consume, `false` otherwise.
    ///
    /// Contracts: `n == 0` returns `true` with no state change; `n > capacity`
    /// returns `false` immediately and forever (unsatisfiable by design — do
    /// not spin on it); on a `false` return a benign refill may have occurred
    /// but the consume is skipped.
    pub fn try_acquire(&self, n: u64) -> bool {
        if n == 0 {
            return true;
        }
        let cost = (n as u128) << SCALE_BITS;
        if cost > self.capacity_scaled {
            // Unsatisfiable forever; short-circuit before locking.
            return false;
        }
        let now = self.clock.now();
        let mut state = self.state.lock().unwrap();
        self.refill_locked(&mut state, now);
        if state.tokens_scaled >= cost {
            state.tokens_scaled -= cost;
            true
        } else {
            // Leave tokens untouched: a false return performs no decrement.
            false
        }
    }

    /// Current whole-token count after a refill. **Advisory**: under
    /// concurrency the value is immediately stale, and a true return here does
    /// NOT guarantee a subsequent `try_acquire` succeeds. This is a refilling
    /// reader (a writer on the shared mutex).
    pub fn available(&self) -> u64 {
        let now = self.clock.now();
        let mut state = self.state.lock().unwrap();
        self.refill_locked(&mut state, now);
        (state.tokens_scaled >> SCALE_BITS) as u64
    }

    /// Test/diagnostic accessor for the underlying clock.
    #[cfg(test)]
    pub(crate) fn clock(&self) -> &C {
        &self.clock
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;

    /// Deterministic clock that only advances when told.
    pub(crate) struct ManualClock {
        base: Instant,
        offset_ns: AtomicU64,
    }

    impl ManualClock {
        fn new() -> Self {
            ManualClock {
                base: Instant::now(),
                offset_ns: AtomicU64::new(0),
            }
        }
        fn advance(&self, d: Duration) {
            self.offset_ns
                .fetch_add(d.as_nanos() as u64, Ordering::SeqCst);
        }
        fn advance_ns(&self, ns: u64) {
            self.offset_ns.fetch_add(ns, Ordering::SeqCst);
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> Instant {
            self.base + Duration::from_nanos(self.offset_ns.load(Ordering::SeqCst))
        }
    }

    fn manual(capacity: u64, rate: f64) -> RateLimiter<ManualClock> {
        RateLimiter::try_with_clock(capacity, rate, ManualClock::new())
            .expect("valid rate in test helper")
    }

    // The independent integer oracle: max sub-tokens issuable over a run.
    // Deliberately a separate, dead-simple reference — not a copy of the impl.
    fn max_issuable_scaled(capacity: u64, rate: f64, elapsed_ns: u128) -> u128 {
        let cap_scaled = (capacity as u128) << SCALE_BITS;
        let rate_scaled = (rate * SCALE_F).floor() as u128;
        cap_scaled + rate_scaled * elapsed_ns / NANOS_PER_SEC
    }

    // ---- 1. No-over-issue property, concurrent ----------------------------

    #[test]
    fn no_over_issue_concurrent() {
        let capacity = 1_000u64;
        let rate = 500.0f64;
        let rl = Arc::new(manual(capacity, rate));
        let granted = Arc::new(AtomicU64::new(0)); // sum of whole-token grants
        let stop = Arc::new(AtomicBool::new(false));

        let mut handles = Vec::new();
        for t in 0..8 {
            let rl = Arc::clone(&rl);
            let granted = Arc::clone(&granted);
            let stop = Arc::clone(&stop);
            handles.push(std::thread::spawn(move || {
                // Deterministic per-thread xorshift for n in 1..=4.
                let mut seed = (t as u64).wrapping_mul(2_654_435_761).wrapping_add(1);
                let mut local: u64 = 0;
                while !stop.load(Ordering::Relaxed) {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    let n = 1 + (seed % 4);
                    if rl.try_acquire(n) {
                        local += n;
                    }
                }
                granted.fetch_add(local, Ordering::SeqCst);
            }));
        }

        // Step the clock forward in known increments while threads hammer.
        let steps = 200u64;
        let step_ns = 100_000u64; // 0.1ms each → 20ms simulated
        for _ in 0..steps {
            rl.clock().advance_ns(step_ns);
            std::thread::yield_now();
        }
        std::thread::sleep(Duration::from_millis(20));
        stop.store(true, Ordering::Relaxed);
        for h in handles {
            h.join().unwrap();
        }

        let total_elapsed_ns = (steps as u128) * (step_ns as u128);
        let granted_whole = granted.load(Ordering::SeqCst);
        let granted_scaled = (granted_whole as u128) << SCALE_BITS;
        let bound = max_issuable_scaled(capacity, rate, total_elapsed_ns);
        assert!(
            granted_scaled <= bound,
            "over-issue: granted {granted_whole} whole ({granted_scaled} scaled) > bound {bound}"
        );
        let st = rl.state.lock().unwrap();
        assert!(st.tokens_scaled <= rl.capacity_scaled);
    }

    // ---- 2. Burst-to-capacity across a rate sweep -------------------------

    #[test]
    fn burst_to_capacity_rate_sweep() {
        let capacity = 100u64;
        for &rate in &[3.0, 6.0, 7.0, 9.0, 0.5, 1.0 / 3600.0, std::f64::consts::PI] {
            let rl = manual(capacity, rate);
            assert_eq!(rl.available(), capacity, "fresh bucket full (rate {rate})");
            assert!(
                rl.try_acquire(capacity),
                "burst to capacity must succeed at rate {rate}"
            );
            assert_eq!(rl.available(), 0, "must debit exactly capacity at rate {rate}");
        }
    }

    // ---- 3. Large-value exactness (> 2^53) --------------------------------

    #[test]
    fn large_value_exactness() {
        let capacity = 1u64 << 54;
        let n = (1u64 << 53) + 1;
        let rl = manual(capacity, 1.0);
        let before = rl.available();
        assert_eq!(before, capacity);
        assert!(rl.try_acquire(n), "should grant exactly 2^53+1 from a 2^54 bucket");
        let after = rl.available();
        assert_eq!(before - after, n, "debit must be exact, no f64 rounding leak");
    }

    // ---- 4. Sub-1-token/sec refill actually recovers ----------------------

    #[test]
    fn sub_one_token_per_sec_recovers() {
        // 0.5 tokens/sec: drain, then accrue.
        let rl = manual(2, 0.5);
        assert!(rl.try_acquire(2));
        assert_eq!(rl.available(), 0);
        rl.clock().advance(Duration::from_secs(1)); // +0.5 token
        assert_eq!(rl.available(), 0, "half a token is not yet whole");
        rl.clock().advance(Duration::from_secs(1)); // +0.5 → 1.0
        assert_eq!(rl.available(), 1, "two seconds at 0.5/s yields one token");

        // 1 token/hour: must eventually recover, not wedge at zero. Because the
        // rate is floored toward under-issue, a whole token lands a hair after
        // the nominal hour — the contract is "eventually recovers", not "exact".
        let rl = manual(1, 1.0 / 3600.0);
        assert!(rl.try_acquire(1));
        assert_eq!(rl.available(), 0);
        rl.clock().advance(Duration::from_secs(1800));
        assert_eq!(rl.available(), 0, "half an hour is not yet a whole token");
        rl.clock().advance(Duration::from_secs(1800)); // ~one hour
        rl.clock().advance(Duration::from_secs(1)); // cover the floor deficit
        assert_eq!(rl.available(), 1, "bucket recovers — does not wedge at zero");
    }

    // ---- 5. High-frequency poll does not starve refill --------------------

    #[test]
    fn high_freq_poll_does_not_starve() {
        let rate = 0.5f64;
        let total_ns = 4_000_000_000u64; // 4s → 2 tokens at 0.5/s

        // Reference: one-shot accrual.
        let oneshot = manual(10, rate);
        assert!(oneshot.try_acquire(10));
        oneshot.clock().advance_ns(total_ns);
        let oneshot_avail = oneshot.available();

        // Polled in 10_000 tiny increments.
        let polled = manual(10, rate);
        assert!(polled.try_acquire(10));
        let increments = 10_000u64;
        let per = total_ns / increments;
        for _ in 0..increments {
            polled.clock().advance_ns(per);
            let _ = polled.available(); // refill on every poll
        }
        polled.clock().advance_ns(total_ns - per * increments); // any remainder
        let polled_avail = polled.available();

        assert_eq!(
            polled_avail, oneshot_avail,
            "polled accrual must equal one-shot accrual (remainder preserved)"
        );
        assert_eq!(oneshot_avail, 2, "4s at 0.5/s should yield exactly 2 tokens");
    }

    // ---- 6. new() input validation ----------------------------------------

    #[test]
    fn new_input_validation_rejects_bad_rates() {
        for &bad in &[f64::NAN, f64::INFINITY, -1.0, 0.0] {
            assert_eq!(
                RateLimiter::try_new(10, bad).err(),
                Some(RateError::NotPositiveFinite),
                "rate {bad} must be rejected"
            );
            let r = std::panic::catch_unwind(|| RateLimiter::new(10, bad));
            assert!(r.is_err(), "RateLimiter::new({bad}) must panic");
        }
        // A rate that floors to zero sub-tokens/sec is rejected.
        let too_small = 1.0 / SCALE_F / 2.0; // < 1 sub-token/sec
        assert_eq!(
            RateLimiter::try_new(10, too_small).err(),
            Some(RateError::TooSmall)
        );
    }

    // ---- 7. n > capacity --------------------------------------------------

    #[test]
    fn n_greater_than_capacity_is_false_deterministically() {
        let rl = manual(5, 1000.0);
        for _ in 0..5 {
            assert!(!rl.try_acquire(6), "n > capacity must be false");
            assert_eq!(rl.available(), 5, "state unchanged by unsatisfiable request");
        }
        rl.clock().advance(Duration::from_secs(100));
        assert!(!rl.try_acquire(6), "still unsatisfiable after refill time");
    }

    // ---- 8. n == 0 --------------------------------------------------------

    #[test]
    fn n_zero_is_true_no_change() {
        let rl = manual(3, 1.0);
        assert!(rl.try_acquire(0));
        assert_eq!(rl.available(), 3);
        assert!(rl.try_acquire(3));
        assert!(rl.try_acquire(0), "zero acquire on empty bucket still true");
        assert_eq!(rl.available(), 0);
    }

    // ---- 9. capacity == 0 -------------------------------------------------

    #[test]
    fn capacity_zero_rejects_everything() {
        let rl = manual(0, 1000.0);
        assert_eq!(rl.available(), 0);
        assert!(rl.try_acquire(0), "n==0 still true");
        for n in 1..=5 {
            assert!(!rl.try_acquire(n), "capacity 0 must reject n={n}");
        }
        rl.clock().advance(Duration::from_secs(10));
        assert!(!rl.try_acquire(1), "no time fills a zero-capacity bucket");
        assert_eq!(rl.available(), 0);
    }

    // ---- 10. Non-advancing clock ------------------------------------------

    #[test]
    fn non_advancing_clock_is_noop() {
        let rl = manual(10, 5.0);
        assert!(rl.try_acquire(4));
        for _ in 0..1000 {
            assert_eq!(rl.available(), 6, "no credit without time advance");
        }
    }

    // ---- 11. Apparent backward step ---------------------------------------

    #[test]
    fn backward_clock_no_negative_credit() {
        let rl = manual(10, 5.0);
        rl.clock().advance(Duration::from_secs(10)); // far forward
        assert!(rl.try_acquire(10)); // full, drain
        assert_eq!(rl.available(), 0);
        // Move "backward" by resetting offset below the current anchor.
        rl.clock().offset_ns.store(0, Ordering::SeqCst);
        assert_eq!(rl.available(), 0, "no negative credit, no panic");
        assert!(!rl.try_acquire(1));
    }

    // ---- 12. Idle-then-burst caps exactly at capacity ---------------------

    #[test]
    fn idle_then_burst_caps_at_capacity() {
        let rl = manual(50, 10.0);
        assert!(rl.try_acquire(50)); // drain
        assert_eq!(rl.available(), 0);
        rl.clock().advance(Duration::from_secs(3600)); // overfill window
        assert_eq!(rl.available(), 50, "overflow forfeited — caps at capacity");
        assert!(rl.try_acquire(50), "exactly one full burst");
        assert!(!rl.try_acquire(1), "immediate second burst is rate-gated");
        rl.clock().advance(Duration::from_secs(1)); // +10 tokens
        assert_eq!(rl.available(), 10);
    }

    // ---- 13. available() ↔ try_acquire quiescent consistency --------------

    #[test]
    fn quiescent_consistency() {
        let rl = manual(20, 3.0);
        assert!(rl.try_acquire(7));
        let k = rl.available();
        assert_eq!(k, 13);
        // Probe k+1 first so we don't perturb the level before the k probe.
        assert!(!rl.try_acquire(k + 1), "k+1 must fail");
        assert!(rl.try_acquire(k), "k must succeed");
        assert_eq!(rl.available(), 0);
    }

    // ---- 14. No drift over long uptime ------------------------------------

    #[test]
    fn no_drift_over_long_uptime() {
        let capacity = 1000u64;
        let rate = 250.0f64;
        let rl = manual(capacity, rate);
        let mut total_elapsed_ns: u128 = 0;
        let mut granted_scaled: u128 = 0;

        for i in 0..2_000_000u64 {
            let step_ns = 500u64; // 0.5µs each
            rl.clock().advance_ns(step_ns);
            total_elapsed_ns += step_ns as u128;
            if i % 3 == 0 && rl.try_acquire(1) {
                granted_scaled += 1u128 << SCALE_BITS;
            }
            let st = rl.state.lock().unwrap();
            assert!(st.tokens_scaled <= rl.capacity_scaled);
        }

        let bound = max_issuable_scaled(capacity, rate, total_elapsed_ns);
        assert!(
            granted_scaled <= bound,
            "drift/over-issue: granted {granted_scaled} > bound {bound}"
        );
        let st = rl.state.lock().unwrap();
        assert!(st.tokens_scaled <= rl.capacity_scaled);
    }
}
