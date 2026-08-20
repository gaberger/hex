//! ADR-056 Frontend Hexagonal Architecture Checker
//!
//! Checks frontend code for hexagonal boundary compliance:
//! F1: Single entry point (1 HTML file in assets/)
//! F2: Store purity (no fetch/WebSocket in stores/)
//! F3: Component fetch-free (no fetch/WebSocket in components/)
//! F5: No inline styles in components (style={)
//! F7: Service singletons exist (services/ directory present)
//! F9: No hardcoded hex colors in components (#RRGGBB patterns)

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Result of a single frontend architecture rule check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendRuleResult {
    /// Rule identifier: "F1", "F2", etc.
    pub id: String,
    /// Human-readable rule name.
    pub name: String,
    /// Whether the rule passed (no violations found).
    pub passed: bool,
    /// Specific violations found for this rule.
    pub violations: Vec<FrontendViolation>,
}

/// A single frontend architecture violation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendViolation {
    /// Project-relative file path.
    pub file: String,
    /// Line number (1-based) where the violation was found.
    pub line: usize,
    /// Human-readable description of what was found.
    pub message: String,
}

/// Complete result of all frontend architecture checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendCheckResult {
    /// Per-rule results.
    pub rules: Vec<FrontendRuleResult>,
    /// Overall frontend health score (0–100).
    pub score: u32,
}

/// Patterns that indicate direct network calls in stores or components.
/// These are unambiguous — method names like `.get()` are excluded to avoid
/// false positives on Map/Set/Array usage.
const FETCH_PATTERNS: &[&str] = &[
    "fetch(",
    "new WebSocket(",
    "new EventSource(",
    "XMLHttpRequest",
];

/// Run all frontend hexagonal architecture checks.
///
/// `root` is the project root directory. The checker looks for `assets/src/`
/// (or the provided `frontend_path` relative to root). If the frontend
/// directory does not exist, returns `None` — the check is skipped gracefully.
pub fn check_frontend(root: &Path) -> Option<FrontendCheckResult> {
    let assets_dir = root.join("assets");
    let src_dir = assets_dir.join("src");

    if !src_dir.is_dir() {
        return None;
    }

    let mut rules = Vec::new();

    rules.push(check_f1_single_entry_point(&assets_dir, root));
    rules.push(check_f2_store_purity(&src_dir, root));
    rules.push(check_f3_component_fetch_free(&src_dir, root));
    rules.push(check_f5_no_inline_styles(&src_dir, root));
    rules.push(check_f7_services_exist(&src_dir));
    rules.push(check_f9_no_hardcoded_colors(&src_dir, root));

    let score = compute_score(&rules);

    Some(FrontendCheckResult { rules, score })
}

/// Compute overall score from rule results.
///
/// Start at 100, subtract per-violation penalties:
/// - F2 (store purity): -3 per violation
/// - F3 (component fetch): -2 per violation
/// - F5 (inline styles): -1 per violation
/// - F9 (hardcoded colors): -1 per violation
/// - F1 (entry point): -10 if failed
/// - F7 (services exist): -5 if failed
fn compute_score(rules: &[FrontendRuleResult]) -> u32 {
    let mut score: i32 = 100;

    for rule in rules {
        let penalty: i32 = match rule.id.as_str() {
            "F1" => {
                if rule.passed {
                    0
                } else {
                    10
                }
            }
            "F2" => rule.violations.len() as i32 * 3,
            "F3" => rule.violations.len() as i32 * 2,
            "F5" => rule.violations.len() as i32,
            "F7" => {
                if rule.passed {
                    0
                } else {
                    5
                }
            }
            "F9" => rule.violations.len() as i32,
            _ => 0,
        };
        score -= penalty;
    }

    score.max(0) as u32
}

// ── F1: Single entry point ──────────────────────────────────────

/// Check that there is exactly one HTML file in assets/ (the single entry point).
fn check_f1_single_entry_point(assets_dir: &Path, root: &Path) -> FrontendRuleResult {
    let mut html_files = Vec::new();
    collect_files_with_extension(assets_dir, "html", &mut html_files);

    let passed = html_files.len() == 1;
    let violations = if passed {
        vec![]
    } else if html_files.is_empty() {
        vec![FrontendViolation {
            file: rel_path(assets_dir, root),
            line: 0,
            message: "No HTML entry point found in assets/".to_string(),
        }]
    } else {
        html_files
            .iter()
            .map(|f| FrontendViolation {
                file: rel_path(f, root),
                line: 0,
                message: format!(
                    "Multiple HTML files found — expected single entry point, got {}",
                    html_files.len()
                ),
            })
            .collect()
    };

    FrontendRuleResult {
        id: "F1".to_string(),
        name: "Single entry point".to_string(),
        passed,
        violations,
    }
}

// ── F2: Store purity ────────────────────────────────────────────

/// Check that store files (in stores/ or store/) contain no fetch/WebSocket calls.
fn check_f2_store_purity(src_dir: &Path, root: &Path) -> FrontendRuleResult {
    let mut violations = Vec::new();

    for dir_name in &["stores", "store"] {
        let store_dir = src_dir.join(dir_name);
        if store_dir.is_dir() {
            let mut files = Vec::new();
            collect_source_files(&store_dir, &mut files);
            for file in &files {
                scan_file_for_patterns(file, root, FETCH_PATTERNS, &mut violations);
            }
        }
    }

    FrontendRuleResult {
        id: "F2".to_string(),
        name: "Store purity (no fetch/WebSocket)".to_string(),
        passed: violations.is_empty(),
        violations,
    }
}

// ── F3: Component fetch-free ────────────────────────────────────

/// Check that component files contain no direct fetch/WebSocket calls.
fn check_f3_component_fetch_free(src_dir: &Path, root: &Path) -> FrontendRuleResult {
    let mut violations = Vec::new();

    let components_dir = src_dir.join("components");
    if components_dir.is_dir() {
        let mut files = Vec::new();
        collect_source_files(&components_dir, &mut files);
        for file in &files {
            scan_file_for_patterns(file, root, FETCH_PATTERNS, &mut violations);
        }
    }

    FrontendRuleResult {
        id: "F3".to_string(),
        name: "Component fetch-free (no direct network calls)".to_string(),
        passed: violations.is_empty(),
        violations,
    }
}

// ── F5: No inline styles ────────────────────────────────────────

/// Check that component files do not use inline styles (style={...}).
fn check_f5_no_inline_styles(src_dir: &Path, root: &Path) -> FrontendRuleResult {
    let mut violations = Vec::new();

    let components_dir = src_dir.join("components");
    if components_dir.is_dir() {
        let mut files = Vec::new();
        collect_source_files(&components_dir, &mut files);
        for file in &files {
            if let Ok(content) = std::fs::read_to_string(file) {
                for (line_num, line) in content.lines().enumerate() {
                    let trimmed = line.trim();
                    // Skip comments
                    if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                        continue;
                    }
                    if line.contains("style={") {
                        violations.push(FrontendViolation {
                            file: rel_path(file, root),
                            line: line_num + 1,
                            message: "Inline style found — use CSS classes instead".to_string(),
                        });
                    }
                }
            }
        }
    }

    FrontendRuleResult {
        id: "F5".to_string(),
        name: "No inline styles in components".to_string(),
        passed: violations.is_empty(),
        violations,
    }
}

// ── F7: Services directory exists ───────────────────────────────

/// Check that a services/ directory exists under src/, indicating proper
/// separation of network/API concerns from stores and components.
fn check_f7_services_exist(src_dir: &Path) -> FrontendRuleResult {
    let services_dir = src_dir.join("services");
    let passed = services_dir.is_dir();

    let violations = if passed {
        vec![]
    } else {
        vec![FrontendViolation {
            file: "assets/src/services/".to_string(),
            line: 0,
            message: "No services/ directory found — network calls should be in service singletons"
                .to_string(),
        }]
    };

    FrontendRuleResult {
        id: "F7".to_string(),
        name: "Service singletons exist".to_string(),
        passed,
        violations,
    }
}

// ── F9: No hardcoded hex colors ─────────────────────────────────

/// Check that component files do not contain hardcoded hex color literals
/// (e.g. `#FF0000`, `#f0f0f0`). Colors should come from CSS variables or
/// a design-token system.
fn check_f9_no_hardcoded_colors(src_dir: &Path, root: &Path) -> FrontendRuleResult {
    let mut violations = Vec::new();

    let components_dir = src_dir.join("components");
    if components_dir.is_dir() {
        let mut files = Vec::new();
        collect_source_files(&components_dir, &mut files);
        for file in &files {
            if let Ok(content) = std::fs::read_to_string(file) {
                for (line_num, line) in content.lines().enumerate() {
                    let trimmed = line.trim();
                    // Skip comments
                    if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                        continue;
                    }
                    if let Some(violation_msg) = find_hex_color(line) {
                        violations.push(FrontendViolation {
                            file: rel_path(file, root),
                            line: line_num + 1,
                            message: violation_msg,
                        });
                    }
                }
            }
        }
    }

    FrontendRuleResult {
        id: "F9".to_string(),
        name: "No hardcoded hex colors in components".to_string(),
        passed: violations.is_empty(),
        violations,
    }
}

/// Check if a line contains a hex color literal (#RGB, #RRGGBB, or #RRGGBBAA).
/// Returns a violation message if found, or None.
fn find_hex_color(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'#' && i + 1 < len {
            // Count consecutive hex digits after #
            let start = i;
            let mut hex_len = 0;
            let mut j = i + 1;
            while j < len && is_hex_digit(bytes[j]) {
                hex_len += 1;
                j += 1;
            }

            // Valid hex color lengths: 3 (#RGB), 4 (#RGBA), 6 (#RRGGBB), 8 (#RRGGBBAA)
            if hex_len == 3 || hex_len == 4 || hex_len == 6 || hex_len == 8 {
                // Make sure this isn't part of an identifier (e.g. #id-selector)
                // After the hex digits, the next char should NOT be a letter or digit
                let after_valid = j >= len || !bytes[j].is_ascii_alphanumeric();
                if after_valid {
                    let color = &line[start..j];
                    return Some(format!(
                        "Hardcoded hex color `{}` — use CSS variable or design token",
                        color
                    ));
                }
            }

            i = j;
        } else {
            i += 1;
        }
    }

    None
}

fn is_hex_digit(b: u8) -> bool {
    b.is_ascii_hexdigit()
}

// ── Utility functions ───────────────────────────────────────────

/// Scan a file for any of the given string patterns, adding violations.
fn scan_file_for_patterns(
    file: &Path,
    root: &Path,
    patterns: &[&str],
    violations: &mut Vec<FrontendViolation>,
) {
    let content = match std::fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return,
    };

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        // Skip comments
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            continue;
        }
        for pattern in patterns {
            if line.contains(pattern) {
                violations.push(FrontendViolation {
                    file: rel_path(file, root),
                    line: line_num + 1,
                    message: format!("Found `{}` — network calls belong in services/", pattern),
                });
                break; // One violation per line is enough
            }
        }
    }
}

/// Recursively collect frontend source files (.ts, .tsx, .js, .jsx, .svelte, .vue).
fn collect_source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_source_files(&path, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            match ext {
                "ts" | "tsx" | "js" | "jsx" | "svelte" | "vue" => {
                    out.push(path);
                }
                _ => {}
            }
        }
    }
}

/// Recursively collect files with a specific extension.
fn collect_files_with_extension(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_with_extension(&path, ext, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            out.push(path);
        }
    }
}

/// Compute a project-relative path string.
fn rel_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper to create a temp project with frontend structure.
    fn setup_temp_project(dir: &Path) {
        let assets = dir.join("assets");
        let src = assets.join("src");
        let stores = src.join("stores");
        let components = src.join("components").join("chat");
        let services = src.join("services");

        fs::create_dir_all(&stores).unwrap();
        fs::create_dir_all(&components).unwrap();
        fs::create_dir_all(&services).unwrap();

        // Single HTML entry point
        fs::write(assets.join("index.html"), "<html></html>").unwrap();

        // Clean store
        fs::write(
            stores.join("chat.ts"),
            r#"
import { createSignal } from 'solid-js';
export const [messages, setMessages] = createSignal([]);
"#,
        )
        .unwrap();

        // Clean component
        fs::write(
            components.join("ChatView.tsx"),
            r#"
import { messages } from '../../stores/chat';
export function ChatView() {
    return <div class="chat">{messages()}</div>;
}
"#,
        )
        .unwrap();
    }

    #[test]
    fn no_frontend_returns_none() {
        let dir = std::env::temp_dir().join("hex-fe-test-none");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(check_frontend(&dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clean_project_scores_100() {
        let dir = std::env::temp_dir().join("hex-fe-test-clean");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        setup_temp_project(&dir);

        let result = check_frontend(&dir).unwrap();
        assert_eq!(result.score, 100);
        for rule in &result.rules {
            assert!(rule.passed, "Rule {} failed: {:?}", rule.id, rule.violations);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn f1_multiple_html_files_fails() {
        let dir = std::env::temp_dir().join("hex-fe-test-f1");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        setup_temp_project(&dir);

        // Add a second HTML file
        fs::write(dir.join("assets").join("chat.html"), "<html></html>").unwrap();

        let result = check_frontend(&dir).unwrap();
        let f1 = result.rules.iter().find(|r| r.id == "F1").unwrap();
        assert!(!f1.passed);
        assert_eq!(f1.violations.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn f2_fetch_in_store_fails() {
        let dir = std::env::temp_dir().join("hex-fe-test-f2");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        setup_temp_project(&dir);

        fs::write(
            dir.join("assets/src/stores/chat.ts"),
            "const data = await fetch('/api/messages');\n",
        )
        .unwrap();

        let result = check_frontend(&dir).unwrap();
        let f2 = result.rules.iter().find(|r| r.id == "F2").unwrap();
        assert!(!f2.passed);
        assert_eq!(f2.violations.len(), 1);
        assert!(result.score <= 97); // -3 per F2 violation
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn f5_inline_style_detected() {
        let dir = std::env::temp_dir().join("hex-fe-test-f5");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        setup_temp_project(&dir);

        fs::write(
            dir.join("assets/src/components/chat/ChatView.tsx"),
            r#"export function ChatView() {
    return <div style={{ color: 'red' }}>hello</div>;
}"#,
        )
        .unwrap();

        let result = check_frontend(&dir).unwrap();
        let f5 = result.rules.iter().find(|r| r.id == "F5").unwrap();
        assert!(!f5.passed);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn f9_hex_color_detected() {
        let dir = std::env::temp_dir().join("hex-fe-test-f9");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        setup_temp_project(&dir);

        fs::write(
            dir.join("assets/src/components/chat/ChatView.tsx"),
            r#"export function ChatView() {
    return <div class="text-[#FF0000]">hello</div>;
}"#,
        )
        .unwrap();

        let result = check_frontend(&dir).unwrap();
        let f9 = result.rules.iter().find(|r| r.id == "F9").unwrap();
        assert!(!f9.passed);
        assert!(f9.violations[0].message.contains("#FF0000"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn f7_no_services_dir_fails() {
        let dir = std::env::temp_dir().join("hex-fe-test-f7");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let assets = dir.join("assets");
        let src = assets.join("src");
        let stores = src.join("stores");
        let components = src.join("components");

        fs::create_dir_all(&stores).unwrap();
        fs::create_dir_all(&components).unwrap();
        fs::write(assets.join("index.html"), "<html></html>").unwrap();
        fs::write(stores.join("app.ts"), "export const x = 1;").unwrap();

        let result = check_frontend(&dir).unwrap();
        let f7 = result.rules.iter().find(|r| r.id == "F7").unwrap();
        assert!(!f7.passed);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hex_color_detection() {
        assert!(find_hex_color("color: #FF0000;").is_some());
        assert!(find_hex_color("color: #f0f;").is_some());
        assert!(find_hex_color("color: #f0f0f0ff;").is_some());
        assert!(find_hex_color("no colors here").is_none());
        // CSS id selectors should not match (letters after hex digits)
        assert!(find_hex_color("#main-header").is_none());
    }
}
