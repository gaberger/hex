//! One line of text in, one line of text out.

use url_shortener_rs::console;
use url_shortener_rs::ports::Console;

/// A tiny xorshift, seeded by a constant, so a failure reproduces on every
/// machine.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// Test 25. The four documented commands give the four documented answers.
#[test]
fn the_four_commands_answer_as_documented() {
    let ui = console();

    let shortened = ui.handle("shorten https://console.test/a");
    let code = shortened.strip_prefix("ok ").expect("shorten answers ok").to_string();

    assert_eq!(ui.handle(&format!("resolve {code}")), "ok https://console.test/a");
    assert_eq!(ui.handle("expire"), "ok 0");
    assert_eq!(ui.handle("wibble"), "err unknown command");
}

/// A code that is well formed but holds nothing is `none`, not an error.
#[test]
fn an_unknown_code_is_none_and_a_bad_code_is_an_error() {
    let ui = console();
    assert_eq!(ui.handle("resolve 0000000"), "none");
    assert!(ui.handle("resolve u").starts_with("err "), "u is refused");
    assert!(ui.handle("shorten not-a-url").starts_with("err "));
}

/// A missing argument is named, not guessed at.
#[test]
fn a_missing_argument_is_reported() {
    let ui = console();
    assert_eq!(ui.handle("shorten"), "err missing argument");
    assert_eq!(ui.handle("resolve"), "err missing argument");
    assert_eq!(ui.handle("shorten   "), "err missing argument");
}

/// Test 26. A thousand generated lines, and never a panic.
#[test]
fn no_generated_line_can_panic() {
    let ui = console();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for round in 0..1_000u32 {
        let length = usize::try_from(rng.next_u64() % 40).unwrap_or(0);
        let line: String = (0..length)
            .map(|_| {
                let byte = u8::try_from(rng.next_u64() % 128).unwrap_or(0);
                char::from(byte)
            })
            .collect();
        let answer = ui.handle(&line);
        assert!(!answer.is_empty(), "round {round} answered nothing");
    }
}
