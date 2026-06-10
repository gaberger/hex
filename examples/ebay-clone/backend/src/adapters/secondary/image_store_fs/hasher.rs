use sha2::{Sha256, Digest};

/// Computes the SHA-256 hash of given bytes.
///
/// # Examples
///
/// ```
/// use adapters::secondary::image_store_fs::hasher::compute_sha256;
/// let data = b"example";
/// assert_eq!(compute_sha256(data), "50d858e..."); // truncated for brevity
/// ```
pub fn compute_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

// docs/specs/ebay-spec-010 ensures this hashing function is used for idempotency checks.