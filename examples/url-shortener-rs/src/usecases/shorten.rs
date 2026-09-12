//! Give an address a code.

use crate::domain::{candidates, CodeWidth, LongUrl, ShortenError, Ttl};
use crate::ports::{Bind, Clock, CodeStore, Shortened};

/// Shorten one address.
///
/// Steps 1, 2 and 4 are pure and finish before any lock exists. Step 5 is the
/// only shared-memory step, and it is one call, so no second thread can act
/// between a read and a write.
///
/// The clock is read exactly once. Two reads can land on opposite sides of the
/// lifetime boundary inside one call, and then the answer contradicts itself.
pub fn shorten(
    store: &dyn CodeStore,
    clock: &dyn Clock,
    ttl: Ttl,
    width: CodeWidth,
    raw: &str,
) -> Result<Shortened, ShortenError> {
    let url = LongUrl::parse(raw).map_err(ShortenError::BadUrl)?;
    let codes = candidates(&url, width);
    let now = clock.now();
    let expires_at = now.plus(ttl);
    match store.bind(&url, &codes, now, expires_at) {
        Bind::Existing(code) => Ok(Shortened { code, created: false }),
        Bind::Created(code) => Ok(Shortened { code, created: true }),
        Bind::Exhausted => Err(ShortenError::CodeSpaceExhausted),
    }
}
