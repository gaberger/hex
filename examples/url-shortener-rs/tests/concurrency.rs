//! Many threads, one book.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Barrier;
use std::thread;
use url_shortener_rs::ports::Shortener;
use url_shortener_rs::{shortener_with, ManualClock, DEFAULT_CODE_WIDTH};

const THREADS: usize = 64;
const TTL: u64 = 1_000_000;

/// Test 22. Sixty-four threads ask for the same address at the same moment.
/// One of them makes the link. All of them get the same ticket.
#[test]
fn one_url_under_many_threads_gets_exactly_one_code() {
    let app = shortener_with(ManualClock::at_millis(0), TTL, DEFAULT_CODE_WIDTH);
    let gate = Barrier::new(THREADS);

    let results: Vec<(String, bool)> = thread::scope(|scope| {
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let app = &app;
                let gate = &gate;
                scope.spawn(move || {
                    gate.wait();
                    let done = app.shorten("https://race.test/one").expect("shortens");
                    (done.code.as_str().to_string(), done.created)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("no panic")).collect()
    });

    let codes: HashSet<&String> = results.iter().map(|(code, _)| code).collect();
    assert_eq!(codes.len(), 1, "all threads must get one code");
    assert_eq!(results.iter().filter(|(_, made)| *made).count(), 1, "exactly one made it");
    assert_eq!(app.live_count(), 1);
}

/// Test 23. Sixty-four threads, sixty-four addresses. Nobody gets anybody
/// else's ticket.
#[test]
fn many_urls_under_many_threads_keep_their_own_codes() {
    let app = shortener_with(ManualClock::at_millis(0), TTL, DEFAULT_CODE_WIDTH);
    let gate = Barrier::new(THREADS);

    let results: Vec<(String, String)> = thread::scope(|scope| {
        let handles: Vec<_> = (0..THREADS)
            .map(|n| {
                let app = &app;
                let gate = &gate;
                scope.spawn(move || {
                    let url = format!("https://race.test/many/{n}");
                    gate.wait();
                    let done = app.shorten(&url).expect("shortens");
                    (done.code.as_str().to_string(), url)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("no panic")).collect()
    });

    let codes: HashSet<&String> = results.iter().map(|(code, _)| code).collect();
    assert_eq!(codes.len(), THREADS, "no code may appear twice");
    for (code, url) in &results {
        let found = app.resolve(code).expect("valid code").expect("live");
        assert_eq!(found.as_str(), url);
    }
    assert_eq!(app.live_count(), THREADS);
}

/// Test 24. Shortening while a sweeper runs.
///
/// A ticket may stop working, because the sweeper thread moves time forward
/// past its death. That is allowed. Handing back somebody else's address is
/// not, and that is what this checks.
#[test]
fn sweeping_never_hands_back_another_threads_url() {
    let clock = ManualClock::at_millis(0);
    let app = shortener_with(clock.clone(), 5, DEFAULT_CODE_WIDTH);
    let stop = AtomicBool::new(false);

    thread::scope(|scope| {
        let sweeper = {
            let clock = clock.clone();
            let app = &app;
            let stop = &stop;
            scope.spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    clock.advance_millis(1);
                    app.expire();
                }
            })
        };

        let workers: Vec<_> = (0..32)
            .map(|n| {
                let app = &app;
                scope.spawn(move || {
                    for round in 0..20 {
                        let url = format!("https://sweep.test/{n}/{round}");
                        let done = app.shorten(&url).expect("shortens");
                        let found = app.resolve(done.code.as_str()).expect("valid code");
                        if let Some(url_back) = found {
                            assert_eq!(url_back.as_str(), url, "a code must never give another address");
                        }
                    }
                })
            })
            .collect();

        for worker in workers {
            worker.join().expect("no panic");
        }
        stop.store(true, Ordering::SeqCst);
        sweeper.join().expect("no panic");
    });
}
