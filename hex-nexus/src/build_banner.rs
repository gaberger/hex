/// Returns a one-line human-readable banner identifying this nexus build.
/// Used by `hex nexus status` and the dashboard header.
pub fn build_banner() -> String {
    format!("hex-nexus {}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_starts_with_hex_nexus_prefix() {
        let banner = build_banner();
        assert!(
            banner.starts_with("hex-nexus "),
            "expected banner to start with 'hex-nexus ', got: {banner:?}"
        );
    }
}
