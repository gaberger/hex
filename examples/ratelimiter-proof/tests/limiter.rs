//! The whole test plan for `ratelimiter-proof`.
//!
//! Every test here uses only the public API. Group A checks capacity, group B
//! checks refill, and group C checks many threads at once.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use ratelimiter_proof::{Clock, ConfigError, ManualClock, TokenBucket};

const SECOND: Duration = Duration::from_secs(1);

// ---------------------------------------------------------------------------
// Group A — capacity
// ---------------------------------------------------------------------------

/// A1. A new bucket is full. This one uses the REAL clock on purpose.
///
/// A fake clock in every test hides the bug where a new bucket starts empty.
/// The real clock is the one every user gets, so we test it.
#[test]
fn a1_new_bucket_is_full_on_the_real_clock() {
    let bucket = TokenBucket::new(10, 5, SECOND).expect("valid config");

    for i in 0..10 {
        assert!(bucket.try_acquire(1), "grant {i} of a full bucket must pass");
    }
    assert!(!bucket.try_acquire(1), "the eleventh grant must fail");
}

/// A2. All or nothing. A call that fails takes nothing with it.
#[test]
fn a2_a_failed_call_consumes_nothing() {
    let bucket = TokenBucket::with_clock(10, 5, SECOND, ManualClock::new()).expect("valid config");

    assert!(!bucket.try_acquire(11), "11 from a bucket of 10 must fail");

    for i in 0..10 {
        assert!(bucket.try_acquire(1), "grant {i} must still pass");
    }
    assert!(!bucket.try_acquire(1), "the bucket is empty now");
}

/// A3. The exact boundary, where the new time lands exactly on now.
#[test]
fn a3_the_exact_boundary() {
    let bucket = TokenBucket::with_clock(10, 5, SECOND, ManualClock::new()).expect("valid config");

    assert!(bucket.try_acquire(10), "the whole bucket in one call must pass");
    assert!(!bucket.try_acquire(1), "nothing is left");
}

/// A4. A huge request cannot poison the limiter.
///
/// Without the `n > capacity` gate the multiply wraps, `empty_at` moves
/// BACKWARDS, and the limiter hands out a trillion tokens and stays broken for
/// ever.
#[test]
fn a4_a_huge_request_cannot_poison_the_limiter() {
    let bucket = TokenBucket::with_clock(10, 5, SECOND, ManualClock::new()).expect("valid config");

    assert!(!bucket.try_acquire(u64::MAX), "u64::MAX must fail, not panic");

    assert!(bucket.try_acquire(10), "the bucket must still be full");
    assert!(!bucket.try_acquire(1), "and no more than full");
}

/// A5. Every constructor error. One assertion for each of the five values.
#[test]
fn a5_every_constructor_error() {
    assert_eq!(
        TokenBucket::new(0, 5, SECOND).unwrap_err(),
        ConfigError::ZeroCapacity
    );

    // This is the divide-by-zero. It returns an error. It does not panic.
    assert_eq!(
        TokenBucket::new(10, 0, SECOND).unwrap_err(),
        ConfigError::ZeroRefillTokens
    );

    assert_eq!(
        TokenBucket::new(10, 5, Duration::ZERO).unwrap_err(),
        ConfigError::ZeroPeriod
    );

    // Two tokens per nanosecond is faster than we can represent.
    assert_eq!(
        TokenBucket::new(10, 2, Duration::from_nanos(1)).unwrap_err(),
        ConfigError::RateTooFast
    );

    // A full bucket of u64::MAX tokens, each worth a second, is far too large.
    assert_eq!(
        TokenBucket::new(u64::MAX, 1, SECOND).unwrap_err(),
        ConfigError::BurstTooLarge
    );
}

/// A6. Zero takes nothing.
#[test]
fn a6_try_acquire_zero_takes_nothing() {
    let bucket = TokenBucket::with_clock(10, 5, SECOND, ManualClock::new()).expect("valid config");

    assert!(bucket.try_acquire(0), "zero tokens are always available");
    assert!(bucket.try_acquire(0), "twice, still free");

    let mut granted = 0;
    while bucket.try_acquire(1) {
        granted += 1;
        assert!(granted <= 10, "the bucket must not grant more than capacity");
    }
    assert_eq!(granted, 10, "a full bucket still grants exactly capacity");
}

/// A7. A burst that overflows a u128 is an error, not a broken limiter.
///
/// The burst check multiplies `capacity` by `nanos_per_token`. The cost of a
/// token comes from `Duration::as_nanos`, which is a u128 and reaches about
/// 2^94, so the product can leave a u128 behind. A5 never sees this, because
/// its largest product is about 1.8e28, far under the ceiling.
///
/// The period below is exactly 2^65 nanoseconds, so one token costs 2^65. At a
/// capacity of 2^63 the product is exactly 2^128. Before the fix this panicked
/// with "attempt to multiply with overflow" in a debug build. In a release
/// build it was worse: the product wrapped to 0, `BurstTooLarge` never came
/// back, and the truncating casts stored a cost of 0 per token. That bucket
/// granted every full-capacity burst for ever, and `approx_available` divided
/// by zero.
#[test]
fn a7_a_burst_that_overflows_u128_is_rejected() {
    // 36_893_488_147 s + 419_103_232 ns == 2^65 ns.
    let period = Duration::new(36_893_488_147, 419_103_232);

    assert_eq!(
        TokenBucket::new(1u64 << 63, 1, period).unwrap_err(),
        ConfigError::BurstTooLarge,
        "a product of 2^128 must be rejected, not wrapped"
    );
}

// ---------------------------------------------------------------------------
// Group B — refill
// ---------------------------------------------------------------------------

/// Drains a bucket to empty, and returns how many single tokens came out.
fn drain<C: Clock>(bucket: &TokenBucket<C>) -> u64 {
    let mut granted = 0;
    while bucket.try_acquire(1) {
        granted += 1;
        assert!(granted < 1_000_000, "drain did not stop; the limiter leaks");
    }
    granted
}

/// B1. Exact refill. Ten capacity, five per second, one second of waiting.
#[test]
fn b1_exact_refill() {
    let clock = Arc::new(ManualClock::new());
    let bucket = TokenBucket::with_clock(10, 5, SECOND, Arc::clone(&clock)).expect("valid config");

    assert_eq!(drain(&bucket), 10, "a new bucket holds capacity");

    clock.advance(SECOND);
    assert_eq!(drain(&bucket), 5, "one second buys exactly five tokens");
    assert!(!bucket.try_acquire(1), "the sixth token is not there yet");
}

/// B2. Time does not bank up past a full bucket.
#[test]
fn b2_the_capacity_clamp() {
    let clock = Arc::new(ManualClock::new());
    let bucket = TokenBucket::with_clock(10, 5, SECOND, Arc::clone(&clock)).expect("valid config");

    assert_eq!(drain(&bucket), 10, "a new bucket holds capacity");

    clock.advance(Duration::from_secs(3600));
    assert_eq!(drain(&bucket), 10, "one hour cannot fill past capacity");
}

/// B3. No time is ever lost.
///
/// Ten thousand steps of one microsecond is ten milliseconds. At 1000 tokens
/// per second that is exactly ten tokens. A design that turns time into tokens
/// and throws the remainder away scores lower. This design stores time, so
/// there is no remainder to throw away.
#[test]
fn b3_no_time_is_ever_lost() {
    let clock = Arc::new(ManualClock::new());
    let bucket = TokenBucket::with_clock(1, 1000, SECOND, Arc::clone(&clock)).expect("valid config");

    let mut granted = 0_u64;
    for _ in 0..10_000 {
        clock.advance(Duration::from_micros(1));
        if bucket.try_acquire(1) {
            granted += 1;
        }
    }

    assert_eq!(granted, 10, "ten milliseconds at 1000/s is exactly ten tokens");
}

/// B4. The rate is never too fast.
///
/// This pins the rounding in both directions. The ceiling rounding makes the
/// limiter a hair slow, never fast.
#[test]
fn b4_the_rate_is_never_too_fast() {
    let clock = Arc::new(ManualClock::new());
    let bucket = TokenBucket::with_clock(1000, 3, SECOND, Arc::clone(&clock)).expect("valid config");

    assert_eq!(drain(&bucket), 1000, "a new bucket holds capacity");

    clock.advance(SECOND);
    let after_one_second = drain(&bucket);
    assert!(
        after_one_second <= 3,
        "one second must never buy more than three tokens, got {after_one_second}"
    );

    clock.advance(Duration::from_nanos(10));
    let total = after_one_second + drain(&bucket);
    assert!(
        total >= 3,
        "one second plus ten nanoseconds must buy three tokens, got {total}"
    );
}

/// B5. The real clock refills.
///
/// The bounds are loose on purpose, because a real scheduler is not exact.
/// This test exists so that `MonotonicClock` is not shipped untested.
#[test]
fn b5_the_real_clock_refills() {
    let bucket = TokenBucket::new(5, 100, SECOND).expect("valid config");

    let mut drained = 0;
    while bucket.try_acquire(1) {
        drained += 1;
        assert!(drained <= 5, "a real-clock drain must stop near capacity");
    }

    thread::sleep(Duration::from_millis(50));

    let mut granted = 0;
    while bucket.try_acquire(1) {
        granted += 1;
        if granted > 10 {
            break;
        }
    }

    assert!(
        (1..=10).contains(&granted),
        "50 ms at 100 tokens/s must grant between 1 and 10, got {granted}"
    );
}

// ---------------------------------------------------------------------------
// Group C — concurrency
// ---------------------------------------------------------------------------

/// C1. The exact drain. This is the strongest test in the suite.
///
/// The clock is frozen, so there is no refill, so the answer is a hard
/// integer. Three details make it strong:
///
/// 1. The `Barrier` starts all 16 threads together. Without it, thread 0 can
///    drain the whole bucket before thread 15 starts, and the test measures
///    nothing.
/// 2. Capacity is HALF the 16,000 attempts, so the compare-and-swap stays
///    contended for the whole run.
/// 3. There are more attempts than capacity, so the test catches both
///    directions. A double grant goes above 8000. A wrong refusal goes below.
#[test]
fn c1_the_exact_drain() {
    const THREADS: usize = 16;
    const PER_THREAD: usize = 1000;
    const CAPACITY: u64 = 8000;

    for round in 0..20 {
        let bucket = Arc::new(
            TokenBucket::with_clock(CAPACITY, 1, SECOND, ManualClock::new()).expect("valid config"),
        );
        let granted = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(THREADS));

        let workers: Vec<_> = (0..THREADS)
            .map(|_| {
                let bucket = Arc::clone(&bucket);
                let granted = Arc::clone(&granted);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    let mut mine = 0;
                    for _ in 0..PER_THREAD {
                        if bucket.try_acquire(1) {
                            mine += 1;
                        }
                    }
                    granted.fetch_add(mine, Ordering::Relaxed);
                })
            })
            .collect();

        for worker in workers {
            worker.join().expect("no worker may panic");
        }

        assert_eq!(
            granted.load(Ordering::Relaxed),
            CAPACITY as usize,
            "round {round}: a frozen clock must grant exactly capacity"
        );
    }
}

/// C2. A moving clock, with BOTH bounds.
///
/// The lower bound matters. A limiter that always answers "no" passes an upper
/// bound with a score of zero.
#[test]
fn c2_a_moving_clock_with_both_bounds() {
    const THREADS: usize = 8;
    const PER_THREAD: usize = 10_000;
    const CAPACITY: u64 = 100;
    /// 1000 tokens per second is one token every 1,000,000 nanoseconds.
    const NANOS_PER_TOKEN: u128 = 1_000_000;

    // Read the clock BEFORE we build the bucket. The window we measure must
    // cover the window the bucket sees, or the upper bound is too tight.
    let start = Instant::now();
    let bucket = Arc::new(TokenBucket::new(CAPACITY, 1000, SECOND).expect("valid config"));
    let granted = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(THREADS));

    let workers: Vec<_> = (0..THREADS)
        .map(|_| {
            let bucket = Arc::clone(&bucket);
            let granted = Arc::clone(&granted);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let mut mine = 0;
                for _ in 0..PER_THREAD {
                    if bucket.try_acquire(1) {
                        mine += 1;
                    }
                }
                granted.fetch_add(mine, Ordering::Relaxed);
            })
        })
        .collect();

    for worker in workers {
        worker.join().expect("no worker may panic");
    }

    let elapsed_nanos = start.elapsed().as_nanos();
    let total = granted.load(Ordering::Relaxed) as u128;

    // Round the allowance UP, so the bound never fails on a rounding edge.
    let refilled = elapsed_nanos.div_ceil(NANOS_PER_TOKEN);
    let upper = u128::from(CAPACITY) + refilled;

    assert!(
        total <= upper,
        "granted {total}, but the window allows at most {upper}"
    );
    assert!(
        total >= u128::from(CAPACITY),
        "granted {total}, but a full bucket alone is worth {CAPACITY}"
    );
}

/// C3. The type is `Sync`. The compiler is the assertion.
///
/// A plain shared reference crosses the thread boundary with no `Arc`. If
/// `TokenBucket` were not `Sync`, this would not compile.
#[test]
fn c3_the_type_is_sync() {
    let bucket = TokenBucket::with_clock(64, 1, SECOND, ManualClock::new()).expect("valid config");
    let granted = AtomicUsize::new(0);

    thread::scope(|scope| {
        for _ in 0..4 {
            let bucket: &TokenBucket<ManualClock> = &bucket;
            let granted = &granted;
            scope.spawn(move || {
                for _ in 0..100 {
                    if bucket.try_acquire(1) {
                        granted.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
    });

    assert_eq!(granted.load(Ordering::Relaxed), 64, "a frozen clock is exact");
}

// ---------------------------------------------------------------------------
// Group D — the far end of the clock
// ---------------------------------------------------------------------------

/// D1. A very old clock still refills.
///
/// The old code stopped the virtual clock at `1 << 62` nanoseconds, which is
/// about 146 years. Past that point the bucket drained once and then said
/// "no" for ever, to every caller. The doc comment claimed the opposite.
///
/// The limiter must keep working for as long as the clock underneath it moves.
#[test]
fn d1_a_clock_past_the_old_ceiling_still_refills() {
    let clock = Arc::new(ManualClock::new());
    let bucket = TokenBucket::with_clock(10, 5, SECOND, Arc::clone(&clock)).expect("valid config");

    // Walk past the old 146-year ceiling. `advance` takes a `Duration`, so we
    // take two steps to get there.
    let half = Duration::from_nanos(1u64 << 61);
    clock.advance(half);
    clock.advance(half);
    clock.advance(SECOND);

    assert_eq!(drain(&bucket), 10, "an old bucket is still full");

    clock.advance(SECOND);
    assert_eq!(drain(&bucket), 5, "one second still buys five tokens");

    clock.advance(Duration::from_secs(3600));
    assert_eq!(drain(&bucket), 10, "an hour still cannot fill past capacity");
}

/// D2. When the clock underneath finally stops, the limiter fails closed.
///
/// `ManualClock` stops at the top of a `u64`, after about 584 years. The
/// virtual clock stops with it. From then on there is no refill: the bucket
/// gives out what it holds and then denies everything. That is the safe
/// direction, and it is what the doc comment must say.
#[test]
fn d2_a_stopped_clock_denies_and_never_over_grants() {
    let clock = Arc::new(ManualClock::new());
    let bucket = TokenBucket::with_clock(10, 5, SECOND, Arc::clone(&clock)).expect("valid config");

    clock.advance(Duration::from_nanos(u64::MAX));
    assert_eq!(clock.elapsed_nanos(), u64::MAX, "the clock is at the top");

    assert_eq!(drain(&bucket), 10, "the last tokens still come out");

    clock.advance(Duration::from_secs(3600));
    assert!(!bucket.try_acquire(1), "a stopped clock grants nothing more");
    assert_eq!(bucket.approx_available(), 0, "and it reports nothing left");
}
