//! The code algorithm: FNV-1a, then SplitMix64, then base 32.
//!
//! Written by hand on purpose. `DefaultHasher` does not promise a stable
//! output, and `RandomState` picks a new secret on every process start. Codes
//! would change on every restart and no compiler would warn you.

use crate::domain::code::{encode, CodeWidth, ShortCode, BITS_PER_CHAR};
use crate::domain::url::LongUrl;

/// Published FNV-1a 64 constants.
pub const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// How many codes we offer for one address before we give up.
pub const MAX_ATTEMPTS: u8 = 8;

/// FNV-1a over 64 bits.
///
/// `wrapping_mul` is the algorithm, not an accident. The wrap is what mixes
/// the high bits back down into the low ones.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// The SplitMix64 finaliser. It spreads one changed input bit across all 64
/// output bits, so two addresses that differ by one byte land far apart.
///
/// `wrapping_mul` is the algorithm here too.
pub fn splitmix64_finalise(mut z: u64) -> u64 {
    z ^= z >> 30;
    z = z.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z ^= z >> 27;
    z = z.wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    z
}

/// The codes this address may take, best first.
///
/// Up to `MAX_ATTEMPTS` of them, duplicates removed, order kept. The list
/// depends only on the address and the width, so it is the same on every
/// machine and after every restart. That is where idempotence comes from —
/// there is no index from address back to code anywhere in this system.
pub fn candidates(url: &LongUrl, width: CodeWidth) -> Vec<ShortCode> {
    let base = fnv1a_64(url.as_str().as_bytes());
    let bits = BITS_PER_CHAR * u32::from(width.get());
    let mask = u64::MAX >> (64 - bits);
    let mut out: Vec<ShortCode> = Vec::with_capacity(usize::from(MAX_ATTEMPTS));
    for attempt in 0..MAX_ATTEMPTS {
        // Fold in one more byte: the attempt number.
        let folded = (base ^ u64::from(attempt)).wrapping_mul(FNV_PRIME);
        let code = encode(splitmix64_finalise(folded) & mask, width);
        if !out.contains(&code) {
            out.push(code);
        }
    }
    out
}
