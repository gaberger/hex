//! `ShortCode` — the ticket you get back, and the alphabet it is written in.

use std::fmt;

/// Crockford Base32, in lowercase.
///
/// It leaves out `i`, `l`, `o` and `u`. A person who reads a code aloud
/// confuses those letters. Base 32 is a power of two, so one character is
/// exactly 5 bits: no division, no remainder, no bias.
pub const ALPHABET: &str = "0123456789abcdefghjkmnpqrstvwxyz";

/// Bits carried by one character of the alphabet.
pub const BITS_PER_CHAR: u32 = 5;

/// How many characters a generated code has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeWidth(u8);

impl CodeWidth {
    pub const MIN: u8 = 1;
    pub const MAX: u8 = 12;

    /// `None` outside 1..=12. Twelve characters is 60 bits, which still fits a
    /// `u64` with room to spare.
    pub fn new(chars: u8) -> Option<CodeWidth> {
        if (Self::MIN..=Self::MAX).contains(&chars) {
            Some(CodeWidth(chars))
        } else {
            None
        }
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl Default for CodeWidth {
    fn default() -> Self {
        CodeWidth(7)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeError {
    Empty,
    TooLong,
    ConfusableU { at: usize },
    NotInAlphabet { at: usize, ch: char },
}

impl fmt::Display for CodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodeError::Empty => write!(f, "empty code"),
            CodeError::TooLong => write!(f, "code too long"),
            CodeError::ConfusableU { at } => write!(f, "confusable u at {at}"),
            CodeError::NotInAlphabet { at, ch } => write!(f, "character {ch} at {at} is not a code character"),
        }
    }
}

/// A validated code, always held in its canonical lowercase form.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShortCode(String);

impl ShortCode {
    /// Read a code a person typed.
    ///
    /// The steps, in order: fold ASCII uppercase to lowercase; map `i` and `l`
    /// to `1` and `o` to `0`; reject `u`.
    ///
    /// There is no safe target for `u`, because it looks like `v`. So this
    /// returns a `Result` and does not claim to be total. Crockford Base32
    /// rejects `U` for the same reason.
    pub fn parse(raw: &str) -> Result<ShortCode, CodeError> {
        if raw.is_empty() {
            return Err(CodeError::Empty);
        }
        if raw.chars().count() > usize::from(CodeWidth::MAX) {
            return Err(CodeError::TooLong);
        }
        let mut out = String::with_capacity(raw.len());
        for (at, typed) in raw.chars().enumerate() {
            let folded = match typed.to_ascii_lowercase() {
                'i' | 'l' => '1',
                'o' => '0',
                other => other,
            };
            if folded == 'u' {
                return Err(CodeError::ConfusableU { at });
            }
            if !ALPHABET.contains(folded) {
                return Err(CodeError::NotInAlphabet { at, ch: typed });
            }
            out.push(folded);
        }
        Ok(ShortCode(out))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ShortCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Write the low `5 * width` bits of `value` as `width` characters, most
/// significant first.
pub fn encode(value: u64, width: CodeWidth) -> ShortCode {
    let alphabet: Vec<char> = ALPHABET.chars().collect();
    let places = usize::from(width.get());
    let mut out = String::with_capacity(places);
    for place in (0..places).rev() {
        let shift = BITS_PER_CHAR * u32::try_from(place).unwrap_or(0);
        let digit = usize::try_from((value >> shift) & 31).unwrap_or(0);
        out.push(alphabet[digit]);
    }
    ShortCode(out)
}

/// The inverse of `encode`. Every character of a `ShortCode` is in the
/// alphabet, so this cannot fail.
pub fn decode(code: &ShortCode) -> u64 {
    let mut value = 0u64;
    for ch in code.as_str().chars() {
        let digit = ALPHABET.find(ch).and_then(|i| u64::try_from(i).ok()).unwrap_or(0);
        value = (value << BITS_PER_CHAR) | digit;
    }
    value
}
