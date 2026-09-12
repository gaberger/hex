//! Turn a code back into an address.

use crate::domain::{LongUrl, ShortCode, ShortenError};
use crate::ports::{Clock, CodeStore};

/// Look up one code.
///
/// `Ok(None)` means the code is well formed but nothing live holds it. That is
/// different from `Err`, which means the caller typed something that is not a
/// code at all.
pub fn resolve(
    store: &dyn CodeStore,
    clock: &dyn Clock,
    raw: &str,
) -> Result<Option<LongUrl>, ShortenError> {
    let now = clock.now();
    let code = ShortCode::parse(raw).map_err(ShortenError::BadCode)?;
    Ok(store.lookup(&code, now))
}
