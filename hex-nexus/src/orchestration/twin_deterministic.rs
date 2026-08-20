// This module contains deterministic pre-checks for the twin reviewer.

pub const ALLOWED_PREFIXES: &[&str] = &[
    "docs/", "src/", "tests/", "examples/", "scripts/",
    "hex-nexus/src/", "hex-nexus/tests/", "hex-cli/src/", "hex-cli/tests/",
    "hex-core/src/", "hex-core/tests/", "hex-agent/src/", "hex-agent/tests/",
    "hex-parser/src/", "hex-parser/tests/", "hex-analyzer/src/", "hex-analyzer/tests/",
    "hex-desktop/src/", "hex-desktop/tests/", "hex-nexus/assets/src/", "hex-cli/assets/",
    "spacetime-modules/"
];

pub fn path_allowlisted(path: &str) -> bool {
    ALLOWED_PREFIXES.iter().any(|&prefix| path.starts_with(prefix)) ||
    path == "Cargo.toml" || path.ends_with("/Cargo.toml")
}

pub fn extension_matches_content(path: &str, content: &str) -> bool {
    let extension = match std::path::Path::new(path).extension() {
        Some(ext) => ext.to_string_lossy(),
        None => return true,
    };

    match extension.as_ref() {
        "rs" => content.contains("use") || content.contains("pub") || content.contains("fn") || content.contains("mod") || content.contains("#[") || content.contains("//"),
        "go" => content.contains("package"),
        "ts" => content.contains("export") || content.contains("import") || content.contains("function") || content.contains("const") || content.contains("let"),
        "md" => content.len() < 4096, // Reasonable length for markdown
        "json" => content.starts_with('{') || content.starts_with('['),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_allowlisted() {
        assert!(path_allowlisted("hex-cli/tests/foo.rs"));
        assert!(!path_allowlisted("foo/bar.rs"));
    }

    #[test]
    fn test_extension_matches_content_rust() {
        let content = "fn example() {}";
        assert!(extension_matches_content("example.rs", content));
    }

    #[test]
    fn test_extension_matches_content_toml() {
        let content = "[package]";
        assert!(!extension_matches_content("Cargo.toml", content));
    }
}