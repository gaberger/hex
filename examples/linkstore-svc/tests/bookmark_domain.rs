//! Pure tests for the rules. No disk, no network, no clock.

use linkstore_svc::domain::bookmark::{BookmarkId, Title};
use linkstore_svc::domain::tag::Tag;
use linkstore_svc::domain::url::{NormalisedUrl, UrlError};

fn normalise(raw: &str) -> String {
    NormalisedUrl::parse(raw).expect("should parse").as_str().to_string()
}

#[test]
fn tracking_parameters_are_stripped_and_the_order_of_the_rest_is_kept() {
    let got = normalise("https://example.com/a?utm_source=x&id=7&gclid=y&fbclid=z&b=2");
    assert_eq!(got, "https://example.com/a?id=7&b=2");
}

#[test]
fn the_scheme_and_host_are_lowercased_but_the_path_keeps_its_case() {
    let got = normalise("HTTPS://Example.COM/Path/To/Thing");
    assert_eq!(got, "https://example.com/Path/To/Thing");
}

#[test]
fn a_default_port_is_removed() {
    assert_eq!(normalise("http://x.com:80/a"), "http://x.com/a");
    assert_eq!(normalise("https://x.com:443/a"), "https://x.com/a");
    // A port that is not the default stays.
    assert_eq!(normalise("https://x.com:8443/a"), "https://x.com:8443/a");
}

#[test]
fn dangerous_and_broken_schemes_are_rejected() {
    assert_eq!(NormalisedUrl::parse("javascript:alert(1)"), Err(UrlError::UnsupportedScheme));
    assert_eq!(NormalisedUrl::parse("data:text/html,x"), Err(UrlError::UnsupportedScheme));
    assert_eq!(NormalisedUrl::parse("file:///etc/passwd"), Err(UrlError::UnsupportedScheme));
    assert_eq!(NormalisedUrl::parse("not a url"), Err(UrlError::Malformed));
    assert!(NormalisedUrl::parse("").is_err());
}

#[test]
fn credentials_never_reach_the_store() {
    let got = normalise("https://user:pw@x.com/");
    assert_eq!(got, "https://x.com/");
    assert!(!got.contains("pw"));
    assert!(!got.contains("user"));
}

#[test]
fn normalisation_is_idempotent() {
    let inputs = [
        "https://example.com",
        "HTTPS://Example.COM:443/Path?utm_source=news",
        "http://x.com:80/a?b=1&utm_medium=e",
        "https://x.com/a#install",
        "https://user:pw@x.com/a",
        "https://x.com/a?only=1",
        "https://x.com/a?gclid=1",
        "https://x.com/",
        "https://x.com/a/b/C",
        "https://x.com/a?q=hello%20world",
        "https://x.com/a?q=a+b&r=2",
        "http://sub.Domain.example.com/Deep/Path?id=9&fbclid=k#frag",
    ];
    for raw in inputs {
        let once = normalise(raw);
        let twice = normalise(&once);
        assert_eq!(once, twice, "not idempotent for {raw}");
    }
}

#[test]
fn a_url_whose_only_parameters_were_tracking_loses_the_question_mark() {
    assert_eq!(normalise("https://x.com/a?utm_source=x&fbclid=y"), "https://x.com/a");
}

#[test]
fn the_tag_rule() {
    assert!(Tag::parse("").is_err());
    assert!(Tag::parse("   ").is_err());
    assert!(Tag::parse("bad\u{1}tag").is_err());
    assert!(Tag::parse(&"a".repeat(65)).is_err());
    assert!(Tag::parse(&"a".repeat(64)).is_ok());
    assert_eq!(Tag::parse("  Rust  ").unwrap().as_str(), "rust");
    // Spaces are allowed: this is a tag people really write.
    assert_eq!(Tag::parse("Machine Learning").unwrap().as_str(), "machine learning");
}

#[test]
fn the_title_rule() {
    assert!(Title::parse("").is_err());
    assert!(Title::parse("   ").is_err());
    assert!(Title::parse("bad\ntitle").is_err());
    assert!(Title::parse(&"t".repeat(513)).is_err());
    assert!(Title::parse(&"t".repeat(512)).is_ok());
    assert_eq!(Title::parse("  Hello  ").unwrap().as_str(), "Hello");
}

#[test]
fn the_identifier_rule() {
    assert!(BookmarkId::parse("").is_err());
    assert!(BookmarkId::parse("abc").is_err());
    assert!(BookmarkId::parse("3F2504E0-4F89-41D3-9A0C-0305E82C3301").is_err());
    assert!(BookmarkId::parse("3f2504e0-4f89-41d3-9a0c-0305e82c330").is_err());
    assert!(BookmarkId::parse("3f2504e04f8941d39a0c0305e82c3301aa").is_err());
    let good = "3f2504e0-4f89-41d3-9a0c-0305e82c3301";
    assert_eq!(BookmarkId::parse(good).unwrap().as_str(), good);
}

#[test]
fn the_url_crate_behaviour_is_pinned() {
    // An upgrade that changes this must fail here, loudly, and not silently
    // create a second row for a link that is already saved.
    assert_eq!(normalise("https://x.com"), "https://x.com/");
}
