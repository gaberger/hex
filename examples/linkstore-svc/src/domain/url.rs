//! The URL rule. Pure text work: parse, judge, and tidy. No I/O.
//!
//! One external crate is named here on purpose: `url`. It parses text and
//! touches nothing else, so it crosses no layer. Hand-rolled percent-decoding
//! is a bug factory, and this is the one place the trade is worth making.

use url::Url;

/// Why a piece of text is not a bookmarkable link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlError {
    /// The text is not a URL at all.
    Malformed,
    /// Only `http` and `https` may be stored. This is what rejects
    /// `javascript:`, `data:`, and `file:`.
    UnsupportedScheme,
    /// A URL with no host names no server.
    EmptyHost,
}

impl std::fmt::Display for UrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UrlError::Malformed => write!(f, "not a valid URL"),
            UrlError::UnsupportedScheme => write!(f, "only http and https are allowed"),
            UrlError::EmptyHost => write!(f, "the URL has no host"),
        }
    }
}

/// Keys that identify a campaign rather than a page. Stripping them is what
/// makes the same article shared from two places one bookmark.
const TRACKING_KEYS: &[&str] = &[
    "gclid", "dclid", "fbclid", "msclkid", "mc_cid", "mc_eid", "igshid", "yclid",
];

fn is_tracking(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.starts_with("utm_") || TRACKING_KEYS.contains(&lower.as_str())
}

/// A link that has passed the rule and been tidied into one canonical spelling.
///
/// There is no public constructor from a raw `String` except
/// [`NormalisedUrl::rehydrate`], so holding one of these is proof the rule ran.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NormalisedUrl(String);

impl NormalisedUrl {
    /// Apply the rule. The order of the steps is the rule.
    pub fn parse(raw: &str) -> Result<Self, UrlError> {
        let mut u = Url::parse(raw.trim()).map_err(|_| UrlError::Malformed)?;

        match u.scheme() {
            "http" | "https" => {}
            _ => return Err(UrlError::UnsupportedScheme),
        }

        let host = u.host_str().unwrap_or_default().to_string();
        if host.is_empty() {
            return Err(UrlError::EmptyHost);
        }

        // A password must never reach the database.
        u.set_username("").map_err(|_| UrlError::Malformed)?;
        u.set_password(None).map_err(|_| UrlError::Malformed)?;

        // The scheme and the host are case-insensitive. The path is NOT:
        // lowercasing a path is a real bug that looks like a feature.
        let lower_host = host.to_ascii_lowercase();
        if lower_host != host {
            u.set_host(Some(&lower_host)).map_err(|_| UrlError::Malformed)?;
        }

        let default_port = if u.scheme() == "http" { 80 } else { 443 };
        if u.port() == Some(default_port) {
            u.set_port(None).map_err(|_| UrlError::Malformed)?;
        }

        // Collect first: `query_pairs` borrows the URL that is about to change.
        let survivors: Vec<(String, String)> = u
            .query_pairs()
            .filter(|(k, _)| !is_tracking(k))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();

        if u.query().is_some() {
            if survivors.is_empty() {
                // Nothing survived, so the `?` goes too.
                u.set_query(None);
            } else {
                let mut pairs = u.query_pairs_mut();
                pairs.clear();
                for (key, value) in &survivors {
                    pairs.append_pair(key, value);
                }
                pairs.finish();
            }
        }

        // The fragment stays: `#install` and `#usage` are different bookmarks
        // to the person who saved them.
        Ok(NormalisedUrl(u.to_string()))
    }

    /// Rebuild a value that was already judged, with no checks at all.
    ///
    /// Callers must have already validated this value. Only the storage
    /// adapter may call it. Reading a row is not judging it — if a stricter
    /// rule shipped yesterday, yesterday's healthy rows must still be readable.
    pub fn rehydrate(value: String) -> Self {
        NormalisedUrl(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
