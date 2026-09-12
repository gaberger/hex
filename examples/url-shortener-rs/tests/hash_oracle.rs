//! The oracle. These numbers come from outside this project, so they cannot
//! encode our own misunderstanding of it.
//!
//! This is the answer to the mirror test: the same model writes the code and
//! the test, so a test that only re-states the code proves nothing.

use url_shortener_rs::domain::{
    candidates, decode, encode, fnv1a_64, CodeWidth, LongUrl, ShortCode, ALPHABET,
};

fn width(chars: u8) -> CodeWidth {
    CodeWidth::new(chars).expect("width in range")
}

/// Test 1. The published FNV-1a 64 vectors.
#[test]
fn fnv1a_matches_the_published_vectors() {
    assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a_64(b"foobar"), 0x8594_4171_f739_67e8);
}

/// Test 2. The alphabet is the published Crockford set.
#[test]
fn the_alphabet_is_the_published_crockford_set() {
    assert_eq!(ALPHABET, "0123456789abcdefghjkmnpqrstvwxyz");
    assert_eq!(ALPHABET.chars().count(), 32);
    for confusable in ['i', 'l', 'o', 'u'] {
        assert!(!ALPHABET.contains(confusable), "{confusable} must be absent");
    }
    // No character appears twice, or two numbers would share one code.
    let mut seen: Vec<char> = ALPHABET.chars().collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 32);
}

/// Test 3. The encoder, hand-checked. Five lines of arithmetic a reviewer can
/// repeat: 31 is the top digit, 32 carries into the next place, 1023 is two
/// full digits.
#[test]
fn the_encoder_writes_the_digits_worked_out_by_hand() {
    let two = width(2);
    assert_eq!(encode(0, two).as_str(), "00");
    assert_eq!(encode(1, two).as_str(), "01");
    assert_eq!(encode(31, two).as_str(), "0z");
    assert_eq!(encode(32, two).as_str(), "10");
    assert_eq!(encode(1023, two).as_str(), "zz");
}

/// Test 4. A pin, labelled as a pin.
///
/// This value was taken from a passing run. It guards against a random seed:
/// if somebody swaps the hand-written hash for `DefaultHasher`, this breaks on
/// the next process start. It is not proof that the hash is right — test 1 is.
#[test]
fn a_fixed_url_gives_a_fixed_code() {
    let url = LongUrl::parse("https://example.com/").expect("valid url");
    let first = candidates(&url, width(7)).first().cloned().expect("a candidate");
    assert_eq!(first.as_str(), "8jwrze9");
}

/// The candidate list is stable, distinct, and the right length.
#[test]
fn candidates_are_stable_and_distinct() {
    let url = LongUrl::parse("https://example.com/a").expect("valid url");
    let once = candidates(&url, width(7));
    let twice = candidates(&url, width(7));
    assert_eq!(once, twice, "the same url must always offer the same codes");
    assert_eq!(once.len(), 8, "eight attempts, no duplicates at this width");

    let mut sorted: Vec<ShortCode> = once.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), once.len(), "duplicates must be removed");

    for code in &once {
        assert_eq!(code.as_str().chars().count(), 7);
        assert_eq!(&ShortCode::parse(code.as_str()).expect("own output parses"), code);
    }
}

/// The encoder and the decoder are inverses inside the width.
#[test]
fn encode_and_decode_are_inverses() {
    let six = width(6);
    for value in [0u64, 1, 31, 32, 1023, 1024, 999_999, 1_073_741_823] {
        assert_eq!(decode(&encode(value, six)), value);
    }
}
