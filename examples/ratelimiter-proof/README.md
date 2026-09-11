# ratelimiter-proof

A thread-safe token-bucket rate limiter, in one file, with zero dependencies.

## The picture

Picture a jar of coins with a tap that drips coins into it. Two people can
both look in the jar and both take the last coin. That is the bug.

So we throw the jar away. We keep one sticky note instead. The note holds one
time: **the moment the bucket becomes empty**. From that one number and a look
at the clock, anybody can work out how many tokens the bucket holds. One
number is easy to swap safely. A jar full of coins is not.

The note starts with a time in the past. That is what makes a new bucket full.

## Use

```rust
use std::sync::Arc;
use std::time::Duration;
use ratelimiter_proof::TokenBucket;

let limiter = Arc::new(TokenBucket::new(10, 5, Duration::from_secs(1))?);
if limiter.try_acquire(1) {
    // you have a token
}
```

Share one limiter between threads with an `Arc`. Every method takes `&self`.

## Test

```bash
cargo test
cargo test --release
```

Fourteen tests in three groups: capacity, refill, and many threads at once.
The strongest one is `c1_the_exact_drain`: 16 threads, a frozen clock, 16,000
attempts at a bucket of 8000, and the total must be exactly 8000 on every run.

## A note on `loom`

`loom` is a tool that runs a concurrent test many times, with the threads
interleaved differently each time. It could check the compare-and-swap loop in
`try_acquire` later, behind an optional feature.

Do not add it now, and do not claim that it checks every interleaving:

* It explores a bounded set of interleavings, not all of them.
* It cannot model `Instant`, so it cannot test the real clock.
* A 16-thread test never finishes under it.
