//! When a ticket stops working.
//!
//! A mapping is live when `now < expires_at`. So a lifetime of 100 ms means
//! the link works for 100 ms and not for 101.

use url_shortener_rs::ports::Shortener;
use url_shortener_rs::{shortener_with, ManualClock, DEFAULT_CODE_WIDTH};

const TTL: u64 = 100;
const ONE_HOUR: u64 = 3_600_000;

/// Test 9. The exact edge, from both sides.
#[test]
fn the_link_dies_at_the_instant_it_expires() {
    let clock = ManualClock::at_millis(0);
    let app = shortener_with(clock.clone(), TTL, DEFAULT_CODE_WIDTH);
    let code = app.shorten("https://edge.test/a").expect("shortens").code;

    clock.set_millis(TTL - 1);
    assert!(app.resolve(code.as_str()).expect("valid code").is_some(), "one tick before is live");

    clock.set_millis(TTL);
    assert!(app.resolve(code.as_str()).expect("valid code").is_none(), "at the instant it is dead");

    clock.set_millis(TTL + 1);
    assert!(app.resolve(code.as_str()).expect("valid code").is_none(), "after it stays dead");
}

/// Test 10. A clock that jumps backwards makes more links look alive. A bad
/// clock can never delete your data.
#[test]
fn a_backwards_clock_deletes_nothing() {
    let clock = ManualClock::at_millis(1_000_000);
    let app = shortener_with(clock.clone(), ONE_HOUR, DEFAULT_CODE_WIDTH);
    let code = app.shorten("https://backwards.test/a").expect("shortens").code;

    clock.set_millis(1);
    assert_eq!(app.expire(), 0, "a backwards jump must sweep nothing");
    assert!(app.resolve(code.as_str()).expect("valid code").is_some());
    assert_eq!(app.live_count(), 1);
}

/// Test 11. The store-wipe fault, which this design cannot have.
///
/// The old shape computed `now - ttl` and compared it to a birth time. In the
/// first hour after the epoch that subtraction underflows to a huge number and
/// every row looks expired. Here there is no subtraction at all. The fault was
/// invisible in a debug build and fatal in release, so check it anyway.
#[test]
fn a_clock_near_zero_wipes_nothing() {
    let clock = ManualClock::at_millis(0);
    let app = shortener_with(clock, ONE_HOUR, DEFAULT_CODE_WIDTH);
    let code = app.shorten("https://epoch.test/a").expect("shortens").code;

    assert_eq!(app.expire(), 0, "nothing has died yet");
    assert!(app.resolve(code.as_str()).expect("valid code").is_some());
    assert_eq!(app.live_count(), 1);
}

/// Test 12. A sweep is not counted twice.
#[test]
fn sweeping_twice_removes_nothing_the_second_time() {
    let clock = ManualClock::at_millis(0);
    let app = shortener_with(clock.clone(), TTL, DEFAULT_CODE_WIDTH);
    app.shorten("https://sweep.test/a").expect("shortens");

    clock.set_millis(TTL * 2);
    assert_eq!(app.expire(), 1);
    assert_eq!(app.expire(), 0);
    assert_eq!(app.live_count(), 0);
}

/// A lifetime of zero is not a lifetime. The composition root falls back to
/// the default rather than handing out links that are dead on arrival.
#[test]
fn a_zero_lifetime_falls_back_to_the_default() {
    let clock = ManualClock::at_millis(0);
    let app = shortener_with(clock.clone(), 0, DEFAULT_CODE_WIDTH);
    let code = app.shorten("https://zero.test/a").expect("shortens").code;
    assert!(app.resolve(code.as_str()).expect("valid code").is_some());
}
