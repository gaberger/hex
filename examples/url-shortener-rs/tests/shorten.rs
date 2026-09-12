//! Giving an address a code, and getting the same one back.

use std::collections::HashMap;
use url_shortener_rs::domain::{candidates, CodeWidth, LongUrl, ShortCode, ShortenError, UrlError};
use url_shortener_rs::ports::Shortener;
use url_shortener_rs::{shortener_with, ManualClock, DEFAULT_CODE_WIDTH};

const TTL: u64 = 1_000;

/// One character wide is a 32-code space, so collisions happen on purpose on
/// every run instead of never.
const NARROW: u8 = 1;

fn narrow() -> CodeWidth {
    CodeWidth::new(NARROW).expect("1 is in range")
}

fn candidates_of(raw: &str, width: CodeWidth) -> Vec<ShortCode> {
    candidates(&LongUrl::parse(raw).expect("valid url"), width)
}

/// Test 13. The same address always gets the same code, and the second call
/// says it changed nothing.
#[test]
fn shortening_the_same_url_twice_gives_one_code() {
    let app = shortener_with(ManualClock::at_millis(0), TTL, DEFAULT_CODE_WIDTH);
    let first = app.shorten("https://idem.test/a").expect("shortens");
    let second = app.shorten("https://idem.test/a").expect("shortens");

    assert_eq!(first.code, second.code);
    assert!(first.created, "the first call made the link");
    assert!(!second.created, "the second call made nothing");
    assert_eq!(app.live_count(), 1);
}

/// Test 14. The test that justifies pass one.
///
/// Without a first pass that looks for a live row belonging to this address,
/// step 5 below claims the now-free earlier code and the address ends up with
/// two live codes at once.
#[test]
fn a_url_never_gets_a_second_live_code() {
    let width = narrow();
    let clock = ManualClock::at_millis(0);
    let app = shortener_with(clock.clone(), TTL, NARROW);

    // 1. Find two addresses whose first candidate is the same code.
    let mut seen: HashMap<ShortCode, String> = HashMap::new();
    let mut pair: Option<(String, String)> = None;
    for n in 0..500 {
        let url = format!("https://collide.test/{n}");
        let first = candidates_of(&url, width)[0].clone();
        if let Some(earlier) = seen.get(&first) {
            pair = Some((earlier.clone(), url));
            break;
        }
        seen.insert(first, url);
    }
    let (v, u) = pair.expect("32 codes and 500 addresses must collide");
    let c0 = candidates_of(&v, width)[0].clone();
    let c1 = candidates_of(&u, width)[1].clone();
    assert_eq!(candidates_of(&u, width)[0], c0, "they share a first candidate");

    // 2. At t = 0, v takes c0.
    assert_eq!(app.shorten(&v).expect("shortens").code, c0);

    // 3. At t = ttl - 1, u finds c0 taken and takes its second candidate.
    clock.set_millis(TTL - 1);
    assert_eq!(app.shorten(&u).expect("shortens").code, c1);

    // 4. At t = ttl, v is dead and u is still live.
    clock.set_millis(TTL);
    assert!(app.resolve(c0.as_str()).expect("valid code").is_none(), "v has died");

    // 5. u must keep the code it already holds.
    let again = app.shorten(&u).expect("shortens");
    assert_eq!(again.code, c1, "u must not be handed the freed c0 as a second code");
    assert!(!again.created);
    assert_eq!(app.live_count(), 1, "one address, one live code");
}

/// Test 15. A ticket handed over is never refused at the same instant.
#[test]
fn a_new_code_resolves_immediately() {
    for start in [0, TTL - 1] {
        let app = shortener_with(ManualClock::at_millis(start), TTL, DEFAULT_CODE_WIDTH);
        let url = "https://instant.test/a";
        let done = app.shorten(url).expect("shortens");
        let found = app.resolve(done.code.as_str()).expect("valid code");
        assert_eq!(found.map(|u| u.as_str().to_string()), Some(url.to_string()), "at t = {start}");
    }
}

/// Test 16. A caller that polls cannot keep a link alive by accident.
#[test]
fn repeating_a_shorten_does_not_extend_the_life() {
    let clock = ManualClock::at_millis(0);
    let app = shortener_with(clock.clone(), TTL, DEFAULT_CODE_WIDTH);
    let first = app.shorten("https://poll.test/a").expect("shortens");

    clock.set_millis(TTL / 2);
    let again = app.shorten("https://poll.test/a").expect("shortens");
    assert_eq!(again.code, first.code);
    assert!(!again.created);

    clock.set_millis(TTL);
    assert!(app.resolve(first.code.as_str()).expect("valid code").is_none(), "the original death time stands");
}

/// Test 17. After it dies, shortening it again gives it a fresh life.
#[test]
fn a_dead_link_is_renewed_by_shortening_it_again() {
    let clock = ManualClock::at_millis(0);
    let app = shortener_with(clock.clone(), TTL, DEFAULT_CODE_WIDTH);
    let first = app.shorten("https://renew.test/a").expect("shortens");

    clock.set_millis(TTL + 1);
    let renewed = app.shorten("https://renew.test/a").expect("shortens");
    assert!(renewed.created, "the row was dead, so this made a new one");

    clock.set_millis(TTL + TTL);
    assert!(app.resolve(renewed.code.as_str()).expect("valid code").is_some(), "it lives a fresh lifetime");
    assert_eq!(renewed.code, first.code, "the candidate list did not change");
}

/// Test 18. Forced collisions. Without the width seam a 35-bit space means the
/// probe walk never runs in any test.
#[test]
fn forced_collisions_keep_every_link_correct() {
    let app = shortener_with(ManualClock::at_millis(0), TTL, NARROW);
    let urls: Vec<String> = (0..8).map(|n| format!("https://forced.test/{n}")).collect();

    let mut taken: HashMap<String, String> = HashMap::new();
    for url in &urls {
        let done = app.shorten(url).expect("eight addresses fit in thirty-two codes");
        let code = done.code.as_str().to_string();
        assert!(taken.insert(code, url.clone()).is_none(), "no code may serve two addresses");
    }

    for (code, url) in &taken {
        let found = app.resolve(code).expect("valid code").expect("live");
        assert_eq!(found.as_str(), url, "each code must give back its own address");
    }

    for url in &urls {
        let repeat = app.shorten(url).expect("shortens");
        assert!(!repeat.created, "a repeat makes nothing new");
        assert_eq!(taken.get(repeat.code.as_str()), Some(url));
    }
    assert_eq!(app.live_count(), 8);
}

/// Test 19. Exhaustion is real, and it ends.
#[test]
fn exhaustion_is_temporary_and_overwrites_nothing() {
    let clock = ManualClock::at_millis(0);
    let app = shortener_with(clock.clone(), TTL, NARROW);

    let mut bound: Vec<(String, String)> = Vec::new();
    let mut refused: Vec<String> = Vec::new();
    for n in 0..40 {
        let url = format!("https://full.test/{n}");
        match app.shorten(&url) {
            Ok(done) => bound.push((done.code.as_str().to_string(), url)),
            Err(ShortenError::CodeSpaceExhausted) => refused.push(url),
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    // Only 32 codes exist, so 40 addresses cannot all fit.
    assert!(!refused.is_empty(), "forty addresses cannot fit in thirty-two codes");
    assert!(bound.len() <= 32);

    for (code, url) in &bound {
        let found = app.resolve(code).expect("valid code").expect("still live");
        assert_eq!(found.as_str(), url, "no earlier mapping may be overwritten");
    }

    // Move past the lifetime, free the space, and try a refused address again.
    clock.set_millis(TTL * 2);
    assert_eq!(app.expire(), bound.len(), "every row was dead");
    let retried = refused.first().expect("at least one was refused");
    let done = app.shorten(retried).expect("the refusal was temporary, not a ban");
    assert!(done.created);
}

/// Test 20. Bad input is refused with the right reason, and stores nothing.
#[test]
fn a_bad_url_is_refused_and_stores_nothing() {
    let app = shortener_with(ManualClock::at_millis(0), TTL, DEFAULT_CODE_WIDTH);
    let too_long = format!("https://a.com/{}", "x".repeat(2049 - "https://a.com/".len()));
    assert_eq!(too_long.len(), 2049);

    assert_eq!(app.shorten("").unwrap_err(), ShortenError::BadUrl(UrlError::Empty));
    assert_eq!(
        app.shorten(&too_long).unwrap_err(),
        ShortenError::BadUrl(UrlError::TooLong { bytes: 2049 })
    );
    assert_eq!(
        app.shorten("https://a.com/\u{1}").unwrap_err(),
        ShortenError::BadUrl(UrlError::Control { at: 14 })
    );
    assert_eq!(app.shorten("not-a-url").unwrap_err(), ShortenError::BadUrl(UrlError::NoScheme));

    assert_eq!(app.live_count(), 0, "a refused address stores nothing");
}

/// Test 21. One byte of difference is a different address. We never fold case
/// in a path.
#[test]
fn one_byte_of_difference_is_a_different_url() {
    let app = shortener_with(ManualClock::at_millis(0), TTL, DEFAULT_CODE_WIDTH);
    let upper = app.shorten("https://a.com/A").expect("shortens");
    let lower = app.shorten("https://a.com/a").expect("shortens");

    assert_ne!(upper.code, lower.code);
    let found_upper = app.resolve(upper.code.as_str()).expect("valid").expect("live");
    let found_lower = app.resolve(lower.code.as_str()).expect("valid").expect("live");
    assert_eq!(found_upper.as_str(), "https://a.com/A");
    assert_eq!(found_lower.as_str(), "https://a.com/a");
}
