#![forbid(unsafe_code)]
//! A thread-safe token bucket that keeps one number, not a pile of tokens.
//!
//! # The picture
//!
//! Picture a jar of coins with a tap that drips coins into it. Two people can
//! both look in the jar and both take the last coin. That is the bug.
//!
//! So we throw the jar away. We keep one sticky note instead. The note holds
//! one time: the moment the bucket becomes empty. From that one number and a
//! look at the clock, anybody can work out how many tokens the bucket holds.
//! One number is easy to swap safely. A jar full of coins is not.
//!
//! The note starts with a time in the past. That is what makes a new bucket
//! full.
//!
//! # What this crate promises
//!
//! * In any window the limiter grants at most `capacity + rate * seconds`.
//! * A new bucket is full.
//! * A token is never granted twice.
//! * [`TokenBucket::try_acquire`] never panics, for any `n`, including
//!   `u64::MAX`.
//! * A rejected call writes nothing.
//!
//! # What this crate does not promise
//!
//! * **Fairness by size.** A caller who asks for 100 tokens can lose again and
//!   again to callers who ask for 1. A fix needs a queue, and a queue needs a
//!   lock.
//! * **A perfect "no".** Under heavy load a thread can say "no" a few
//!   microseconds after a token appears. We never grant a token that does not
//!   exist, but a denial can be late.
//! * **Suspend.** The monotonic clock stops while the machine sleeps. After
//!   wake-up the bucket holds fewer tokens than the wall clock suggests. Never
//!   use `SystemTime` here: a time correction can move it backwards and break
//!   every safety argument.
//! * **More than about 10 million grants per second.** All cores pass one word
//!   between them. If you need more, shard into `K` buckets of `N / K` and
//!   `R / K` yourself. That trades exactness for speed, so it stays out of this
//!   crate.
//! * **Many processes.** This limiter lives in one process. It is not a
//!   distributed limiter.
//!
//! # Example
//!
//! ```
//! use std::sync::Arc;
//! use std::time::Duration;
//! use ratelimiter_proof::TokenBucket;
//!
//! let limiter = Arc::new(TokenBucket::new(10, 5, Duration::from_secs(1)).unwrap());
//! assert!(limiter.try_acquire(10));
//! assert!(!limiter.try_acquire(1));
//! ```

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// The largest bucket we can hold, measured in time.
///
/// The constructor rejects anything bigger. The limit keeps every later sum
/// well under `u64::MAX`.
const MAX_BURST_NS: u64 = 1 << 62;

/// A source of time that never moves backwards.
///
/// The bound `Send + Sync` lives on the trait on purpose. Without it the user
/// gets a confusing error at the `Arc` line, far from the cause.
pub trait Clock: Send + Sync {
    /// Nanoseconds since this clock was made. This value must never get
    /// smaller.
    fn elapsed_nanos(&self) -> u64;
}

/// A shared clock is still a clock.
///
/// A test holds one `Arc<ManualClock>` and gives the bucket another. Both
/// point at the same number, so the test can move time and the bucket sees it.
impl<C: Clock + ?Sized> Clock for std::sync::Arc<C> {
    fn elapsed_nanos(&self) -> u64 {
        (**self).elapsed_nanos()
    }
}

/// The real clock. Use this one in production.
///
/// It counts from the moment you make it. It never moves backwards, and a
/// system time correction cannot touch it.
#[derive(Debug)]
pub struct MonotonicClock {
    start: Instant,
}

impl MonotonicClock {
    /// Makes a clock that reads zero right now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn elapsed_nanos(&self) -> u64 {
        // 584 years of nanoseconds do not fit in a u64. Stop at the top
        // instead of wrapping around to zero.
        u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

/// A clock you move by hand. Use this one in tests.
///
/// There is no `set`, and there is no way back. A test that moves time
/// backwards breaks every safety argument in this crate.
#[derive(Debug, Default)]
pub struct ManualClock {
    nanos: AtomicU64,
}

impl ManualClock {
    /// Makes a clock that reads zero and stays there until you move it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            nanos: AtomicU64::new(0),
        }
    }

    /// Moves the clock forward by `d`. Forward is the only direction.
    pub fn advance(&self, d: Duration) {
        let add = u64::try_from(d.as_nanos()).unwrap_or(u64::MAX);
        let _ = self
            .nanos
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |cur| {
                Some(cur.saturating_add(add))
            });
    }
}

impl Clock for ManualClock {
    fn elapsed_nanos(&self) -> u64 {
        self.nanos.load(Ordering::Acquire)
    }
}

/// Every way a bucket can be asked for something we cannot build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// `capacity` was 0. A bucket that holds nothing is not a limiter.
    ZeroCapacity,
    /// `refill_tokens` was 0. This is the divide-by-zero, caught early.
    ZeroRefillTokens,
    /// `refill_period` was 0. A period of no time is not a period.
    ZeroPeriod,
    /// More than one token per nanosecond. We cannot represent that rate.
    RateTooFast,
    /// `capacity * nanos_per_token` does not fit in our safety budget.
    BurstTooLarge,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::ZeroCapacity => "capacity must be at least 1",
            Self::ZeroRefillTokens => "refill_tokens must be at least 1",
            Self::ZeroPeriod => "refill_period must be longer than zero",
            Self::RateTooFast => "rate is faster than one token per nanosecond",
            Self::BurstTooLarge => "capacity times the cost of a token is too large",
        };
        f.write_str(text)
    }
}

impl std::error::Error for ConfigError {}

/// The one number that changes, alone on its own cache line.
///
/// The padding is not there because of the read-only fields next to it.
/// Read-only neighbours cost nothing, because false sharing needs a writer.
/// The real reason is two limiters side by side in a `Vec`, or another hot
/// counter in the same struct. On x86-64 the hardware moves memory in pairs of
/// 64 bytes, so 128 is the right number.
#[derive(Debug)]
#[repr(align(128))]
struct Cell(AtomicU64);

/// A token bucket that many threads share through an `Arc`.
///
/// Every field except [`Cell`] is read-only after you build the object.
#[derive(Debug)]
pub struct TokenBucket<C: Clock = MonotonicClock> {
    /// The virtual nanosecond at which the bucket holds zero tokens.
    empty_at: Cell,
    /// The time cost of one token.
    nanos_per_token: u64,
    /// A full bucket, measured in time. It is `capacity * nanos_per_token`.
    burst_ns: u64,
    /// The token count you asked for.
    capacity: u64,
    /// The clock reading at the moment we built the object.
    origin: u64,
    /// The injected clock.
    clock: C,
}

impl TokenBucket<MonotonicClock> {
    /// Builds a bucket on the real clock.
    ///
    /// The bucket holds `capacity` tokens and gains `refill_tokens` every
    /// `refill_period`. A new bucket is full.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] if the numbers cannot make a working limiter.
    pub fn new(
        capacity: u64,
        refill_tokens: u64,
        refill_period: Duration,
    ) -> Result<Self, ConfigError> {
        Self::with_clock(capacity, refill_tokens, refill_period, MonotonicClock::new())
    }
}

impl<C: Clock> TokenBucket<C> {
    /// Builds a bucket on a clock you supply.
    ///
    /// Use this with [`ManualClock`] in tests. A real clock in a test is slow
    /// and flaky.
    ///
    /// # Errors
    ///
    /// The five checks run in this order, and the first failure wins:
    ///
    /// 1. `capacity == 0` gives [`ConfigError::ZeroCapacity`].
    /// 2. `refill_tokens == 0` gives [`ConfigError::ZeroRefillTokens`].
    /// 3. `refill_period == 0` gives [`ConfigError::ZeroPeriod`].
    /// 4. `refill_tokens > period_nanos` gives [`ConfigError::RateTooFast`].
    /// 5. A burst above `1 << 62` nanoseconds gives
    ///    [`ConfigError::BurstTooLarge`].
    pub fn with_clock(
        capacity: u64,
        refill_tokens: u64,
        refill_period: Duration,
        clock: C,
    ) -> Result<Self, ConfigError> {
        if capacity == 0 {
            return Err(ConfigError::ZeroCapacity);
        }
        if refill_tokens == 0 {
            return Err(ConfigError::ZeroRefillTokens);
        }
        let period_nanos = refill_period.as_nanos();
        if period_nanos == 0 {
            return Err(ConfigError::ZeroPeriod);
        }
        let tokens = u128::from(refill_tokens);
        if tokens > period_nanos {
            return Err(ConfigError::RateTooFast);
        }

        // Round the rate up, never down. Integer division rounds down, and a
        // smaller cost per token makes a FASTER limiter. For a rate limiter an
        // error that lets more traffic through is not an accuracy problem. It
        // is a failure. The cost of rounding up is under one nanosecond per
        // token.
        //
        // All the division in this crate happens here, once, after the zero
        // checks. There is no division at run time.
        let nanos_per_token = period_nanos.div_ceil(tokens);

        // `nanos_per_token` is NOT a u64. `Duration::as_nanos` returns a u128
        // that reaches about 2^94, so a u128 product can and does overflow:
        // 2^63 tokens at 2^65 nanoseconds each is exactly 2^128. So the
        // multiply is checked. An overflow is a burst far above the ceiling,
        // and takes the same exit as a burst that merely passes it.
        let burst = match u128::from(capacity).checked_mul(nanos_per_token) {
            Some(burst) => burst,
            None => return Err(ConfigError::BurstTooLarge),
        };
        if burst > u128::from(MAX_BURST_NS) {
            return Err(ConfigError::BurstTooLarge);
        }

        // The check above bounds both values by `MAX_BURST_NS`, because
        // `capacity >= 1` and `nanos_per_token >= 1`. We still convert rather
        // than cast. A silent `as` truncation here builds a limiter that never
        // limits, so the narrowing must be one the compiler can see.
        let burst_ns = u64::try_from(burst).map_err(|_| ConfigError::BurstTooLarge)?;
        let nanos_per_token =
            u64::try_from(nanos_per_token).map_err(|_| ConfigError::BurstTooLarge)?;

        let origin = clock.elapsed_nanos();
        Ok(Self {
            empty_at: Cell(AtomicU64::new(0)),
            nanos_per_token,
            burst_ns,
            capacity,
            origin,
            clock,
        })
    }

    /// The virtual clock. It starts at `burst_ns`, not at zero.
    ///
    /// Three good things follow from that one offset:
    ///
    /// * A new bucket is full, because `min(burst_ns, now - empty_at)` is
    ///   `min(burst_ns, burst_ns)`.
    /// * `now - burst_ns` can never go below zero, so there is no underflow.
    /// * A clock you built long before the bucket still works, because we
    ///   take away `origin`.
    #[inline]
    fn now(&self) -> u64 {
        let raw = self.clock.elapsed_nanos().saturating_sub(self.origin);
        raw.min(self.max_elapsed_ns()) + self.burst_ns
    }

    /// The largest elapsed time we look at.
    ///
    /// Two bursts of room sit above this number. One of them goes on the
    /// virtual clock in [`Self::now`]. The other is the most that
    /// `try_acquire` can ever add to `empty_at`. So no sum here can overflow.
    ///
    /// The clock underneath stops at the top of a `u64` too, after about 584
    /// years. This ceiling sits `2 * burst_ns` below that top, so an ordinary
    /// bucket reaches the two within seconds of each other. Only the largest
    /// burst we allow pulls the ceiling down far, to about 292 years.
    ///
    /// When the virtual clock stops, the bucket stops refilling: callers take
    /// the last tokens, and every later call gets `false`. That is the safe
    /// direction, because the limiter never hands out a token that time did
    /// not buy. But it is a full stop, not a bucket that stays full.
    #[inline]
    fn max_elapsed_ns(&self) -> u64 {
        // `burst_ns` is at most `MAX_BURST_NS`, so the doubling cannot wrap.
        u64::MAX - 2 * self.burst_ns
    }

    /// Asks for `n` tokens. Returns `true` if you got them.
    ///
    /// A `false` answer takes nothing and writes nothing.
    ///
    /// There is no disk here, so there is no run-time error, so there is no
    /// `Result`. The answer is a plain `bool`.
    ///
    /// # Panics
    ///
    /// Never. Any `n` is safe, including `u64::MAX`.
    #[must_use]
    pub fn try_acquire(&self, n: u64) -> bool {
        if n == 0 {
            // No clock read, and no write. Zero takes nothing.
            return true;
        }
        if n > self.capacity {
            // This gate runs BEFORE the multiply, which is what makes the
            // multiply safe and stops `empty_at` ever moving backwards.
            return false;
        }

        // Cannot overflow: `n <= capacity`, so the product is at most
        // `burst_ns`, which the constructor already checked.
        let need = n * self.nanos_per_token;

        loop {
            // Read the clock again on every turn. A cached time lets a thread
            // say "no" from a stale reading. One extra clock read on a
            // contended retry is cheap.
            let now = self.now();
            let cur = self.empty_at.0.load(Ordering::Relaxed);

            // `now` is always at least `burst_ns`, so this cannot go below
            // zero. We write it as a saturating subtraction anyway.
            let base = cur.max(now.saturating_sub(self.burst_ns));
            let new = base + need;

            if new > now {
                // Not enough tokens. Write nothing, and do not touch the
                // atomic. This keeps the hot path free under overload.
                return false;
            }

            // A compare-and-swap is one machine instruction. It means "change
            // this number to `new`, but only if it still equals `cur`". Only
            // one thread can win a given swap. That is why no token goes out
            // twice.
            match self.empty_at.0.compare_exchange_weak(
                cur,
                new,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(_) => std::hint::spin_loop(),
            }
        }
    }

    /// The token count you asked for when you built the bucket.
    #[must_use]
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// For logging only. Never make a decision from this number.
    ///
    /// The word "approx" is in the name on purpose. The answer is stale the
    /// moment you read it.
    #[must_use]
    pub fn approx_available(&self) -> u64 {
        let now = self.now();
        let cur = self.empty_at.0.load(Ordering::Relaxed);
        let spare = now.saturating_sub(cur).min(self.burst_ns);
        spare / self.nanos_per_token
    }
}
