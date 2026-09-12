//! Reading a code a person typed.

use url_shortener_rs::domain::{encode, CodeError, CodeWidth, ShortCode};

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

/// Test 5. Upper case equals lower case, and the confusable letters fold.
#[test]
fn typed_confusables_fold_to_their_digits() {
    let folded = ShortCode::parse("O0IL1").expect("folds");
    let plain = ShortCode::parse("00111").expect("already canonical");
    assert_eq!(folded, plain);
}

/// Test 6. `u` has no safe target, because it looks like `v`. So it is
/// refused, in either case.
#[test]
fn u_is_refused_in_either_case() {
    assert_eq!(ShortCode::parse("u"), Err(CodeError::ConfusableU { at: 0 }));
    assert_eq!(ShortCode::parse("U"), Err(CodeError::ConfusableU { at: 0 }));
    assert_eq!(ShortCode::parse("12u45"), Err(CodeError::ConfusableU { at: 2 }));
}

/// Test 7. The direction that matters: human input going in, not machine
/// output coming back.
#[test]
fn a_typed_code_renders_as_its_canonical_form() {
    let code = ShortCode::parse("O0IL").expect("folds");
    assert_eq!(code.as_str(), "0011");
}

/// Test 8. Every code the machine writes is a code the parser accepts, and
/// parsing it changes nothing.
#[test]
fn parsing_a_rendered_code_returns_the_same_code() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    for round in 0..10_000u32 {
        let chars = u8::try_from(round % 12).unwrap_or(0) + 1;
        let width = CodeWidth::new(chars).expect("1..=12");
        let code = encode(rng.next_u64(), width);
        let reparsed = ShortCode::parse(code.as_str()).expect("own output must parse");
        assert_eq!(reparsed, code, "round {round}");
    }
}

/// The refusals a caller can hit.
#[test]
fn malformed_codes_are_refused_with_a_reason() {
    assert_eq!(ShortCode::parse(""), Err(CodeError::Empty));
    assert_eq!(ShortCode::parse("0123456789abc"), Err(CodeError::TooLong));
    assert_eq!(ShortCode::parse("0!"), Err(CodeError::NotInAlphabet { at: 1, ch: '!' }));
    // The character reported is the one the caller typed, not our folded form.
    assert_eq!(ShortCode::parse("aé"), Err(CodeError::NotInAlphabet { at: 1, ch: 'é' }));
}
