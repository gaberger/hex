//! UX analyst (workplan `wp-idle-research-swarm`, P3.3).
//!
//! Walks the `hex-nexus/assets/` dashboard source tree and runs a set of
//! deterministic accessibility / consistency checks against every `.tsx`,
//! `.ts`, and `.css` file. Findings are emitted with
//! `domain = Other("ux")` because hex-core does not (yet) carry a first-class
//! UX domain — the open-ended `Other` variant exists exactly for analyst
//! types like this.
//!
//! The analyst itself is intentionally code-first and offline: nothing here
//! calls an LLM. The workplan task description ends with "T2.5 synthesizes
//! top findings", and that synthesis happens *downstream* in the swarm —
//! this module's job is to surface the raw, evidence-backed signals so the
//! synthesis tier has a deterministic substrate to summarize.
//!
//! The IO layer ([`analyze_ux`]) is a thin recursive walker over
//! `assets/src/`. All structural parsing happens in pure functions
//! ([`parse_findings_from_source`] and friends) that take `&str`s, so unit
//! tests don't need a real dashboard tree on disk.
//!
//! ### What we look for
//!
//! | Rule | Severity | Why |
//! |------|----------|-----|
//! | `innerHTML` / `outerHTML` / `insertAdjacentHTML` in primary adapter code | High | XSS + a11y; explicitly forbidden by CLAUDE.md |
//! | `<img …>` without an `alt` attribute | Medium | WCAG 1.1.1 — non-text content needs a text alternative |
//! | Clickable `<div onClick={…}>` without `role`, `aria-label`, or visible text | Medium | Keyboard / screen-reader inaccessible |
//! | `bg-[#…]` / `text-[#…]` / `border-[#…]` arbitrary Tailwind colors | Low | Bypasses the design tokens — visual drift |
//! | Hex / `rgb()` / `hsl()` color literals inside `.css` (outside `:root` token blocks) | Low | Same — visual drift |
//! | Inline `style={{ color: '…' }}` / `style={{ font…: '…' }}` in TSX | Low | Inline styling escapes the token system |
//! | Raw `font-family:` declarations outside the design-token files | Low | Font drift from the dashboard theme |
//!
//! Each rule is a self-contained `detect_*` function on a `(rel_path, line,
//! line_no)` tuple so adding a new rule is a one-function change.

use std::path::{Path, PathBuf};

use hex_core::{ActionKind, Domain, Finding, Severity, SuggestedAction};
use sha2::{Digest, Sha256};

/// Errors produced by [`analyze_ux`].
#[derive(Debug)]
pub enum AnalystError {
    /// Failed to walk the dashboard asset tree.
    Walk {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Failed to read a candidate source file.
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for AnalystError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalystError::Walk { path, source } => {
                write!(f, "failed to walk {}: {}", path.display(), source)
            }
            AnalystError::Read { path, source } => {
                write!(f, "failed to read {}: {}", path.display(), source)
            }
        }
    }
}

impl std::error::Error for AnalystError {}

/// Run the deterministic UX analyst against an `hex-nexus/assets/` tree.
///
/// `assets_root` is expected to point at the directory that contains
/// `src/` (i.e. the same value as `hex-nexus/assets/`). A missing or empty
/// `assets/src/` is not an error — we just return an empty finding set, the
/// same way [`analyze_drift`](crate::research::drift_analyst::analyze_drift)
/// treats a missing `docs/workplans/`.
pub fn analyze_ux(assets_root: &Path) -> Result<Vec<Finding>, AnalystError> {
    let src = assets_root.join("src");
    if !src.exists() {
        return Ok(Vec::new());
    }

    let mut files: Vec<PathBuf> = Vec::new();
    collect_source_files(&src, &mut files)?;
    files.sort();

    let mut findings: Vec<Finding> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in files {
        let contents = std::fs::read_to_string(&path).map_err(|source| AnalystError::Read {
            path: path.clone(),
            source,
        })?;
        let rel = path
            .strip_prefix(assets_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        for f in parse_findings_from_source(&rel, &contents) {
            if seen.insert(f.id.clone()) {
                findings.push(f);
            }
        }
    }

    // Sort severity-desc so a downstream synthesis step ("T2.5 synthesizes top
    // findings") can take the head of the list deterministically.
    findings.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.id.cmp(&b.id)));
    Ok(findings)
}

fn collect_source_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), AnalystError> {
    let entries = std::fs::read_dir(dir).map_err(|source| AnalystError::Walk {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        // Skip vendored / generated trees: they're noisy and not the dashboard's
        // concern.
        if path.is_dir() {
            if matches!(name, "node_modules" | "dist" | "build" | "__tests__" | ".vite") {
                continue;
            }
            collect_source_files(&path, out)?;
            continue;
        }
        if is_ux_source(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn is_ux_source(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("tsx") | Some("ts") | Some("jsx") | Some("js") | Some("css")
    )
}

/// Parse the UX findings out of a single source file.
///
/// `rel_path` is purely cosmetic — it's used in titles and finding ids so the
/// emitted record points back at the right file. `contents` is the raw text
/// of a `.tsx`/`.ts`/`.css` file. Lines are scanned independently, so the
/// caller doesn't need to canonicalize whitespace.
pub fn parse_findings_from_source(rel_path: &str, contents: &str) -> Vec<Finding> {
    let is_css = rel_path.ends_with(".css");
    let is_tsx_like = rel_path.ends_with(".tsx") || rel_path.ends_with(".jsx");
    let mut out: Vec<Finding> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut push = |f: Finding| {
        if seen.insert(f.id.clone()) {
            out.push(f);
        }
    };

    for (idx, raw) in contents.lines().enumerate() {
        let line_no = (idx + 1) as u64;
        let line = raw.trim_end();
        if line.is_empty() {
            continue;
        }
        let stripped = line.trim_start();
        // Skip line comments — drift heuristics fire on actual product code.
        if is_comment_line(stripped, is_css) {
            continue;
        }

        if !is_css {
            if let Some(f) = detect_dangerous_html(rel_path, line_no, line) {
                push(f);
            }
        }
        if is_tsx_like {
            if let Some(f) = detect_img_missing_alt(rel_path, line_no, line) {
                push(f);
            }
            if let Some(f) = detect_clickable_without_label(rel_path, line_no, line) {
                push(f);
            }
            if let Some(f) = detect_arbitrary_tailwind_color(rel_path, line_no, line) {
                push(f);
            }
            if let Some(f) = detect_inline_style_drift(rel_path, line_no, line) {
                push(f);
            }
        }
        if is_css {
            if let Some(f) = detect_css_color_literal(rel_path, line_no, line) {
                push(f);
            }
            if let Some(f) = detect_css_font_drift(rel_path, line_no, line) {
                push(f);
            }
        }
    }

    out
}

fn is_comment_line(stripped: &str, is_css: bool) -> bool {
    if stripped.starts_with("//") {
        return true;
    }
    if stripped.starts_with("/*") || stripped.starts_with('*') {
        return true;
    }
    if is_css && stripped.starts_with("/*") {
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Detectors
// ---------------------------------------------------------------------------

fn detect_dangerous_html(rel_path: &str, line_no: u64, line: &str) -> Option<Finding> {
    let pattern = ["innerHTML", "outerHTML", "insertAdjacentHTML"]
        .into_iter()
        .find(|p| line.contains(p))?;
    // Skip type-only references — `Element['innerHTML']` is fine. We only
    // care about assignment / invocation sites.
    if !line.contains('=') && !line.contains('(') {
        return None;
    }
    let title = format!("{rel_path}:{line_no}: dangerous {pattern}");
    Some(Finding {
        id: stable_id("dangerous-html", &format!("{rel_path}|{line_no}|{pattern}")),
        domain: Domain::Other("ux".into()),
        severity: Severity::High,
        title: truncate(&title, 120),
        evidence: vec![
            format!("{rel_path}:{line_no}: {}", line.trim()),
            format!("rule: no-{}", pattern.to_ascii_lowercase()),
            "CLAUDE.md: primary adapters MUST NOT use innerHTML/outerHTML/insertAdjacentHTML".into(),
        ],
        suggested_action: SuggestedAction {
            kind: ActionKind::DraftWorkplan,
            draft_ref: None,
        },
    })
}

fn detect_img_missing_alt(rel_path: &str, line_no: u64, line: &str) -> Option<Finding> {
    // `<img ` opener with no `alt=` on the same line. Multi-line `<img>`
    // tags are rare in the codebase; if one shows up the next-line scan
    // will not flag it, which is the conservative tradeoff.
    let trimmed = line.trim();
    if !trimmed.contains("<img ") && !trimmed.contains("<img\t") {
        return None;
    }
    if trimmed.contains(" alt=") || trimmed.contains("\talt=") {
        return None;
    }
    let title = format!("{rel_path}:{line_no}: <img> without alt= attribute");
    Some(Finding {
        id: stable_id("img-no-alt", &format!("{rel_path}|{line_no}")),
        domain: Domain::Other("ux".into()),
        severity: Severity::Medium,
        title: truncate(&title, 120),
        evidence: vec![
            format!("{rel_path}:{line_no}: {}", trimmed),
            "rule: a11y/alt-text".into(),
            "WCAG 1.1.1 — non-text content requires a text alternative".into(),
        ],
        suggested_action: SuggestedAction {
            kind: ActionKind::AmendWorkplan,
            draft_ref: None,
        },
    })
}

fn detect_clickable_without_label(rel_path: &str, line_no: u64, line: &str) -> Option<Finding> {
    // `<div onClick=…>` (or span / li) is the classic non-button clickable.
    // We flag when the same line has no `role=`, `aria-label=`, `aria-labelledby=`,
    // and no visible text immediately after the closing `>`.
    let lower = line.to_ascii_lowercase();
    let opens_div_click = (lower.contains("<div ") || lower.contains("<span ") || lower.contains("<li "))
        && lower.contains("onclick=");
    if !opens_div_click {
        return None;
    }
    if lower.contains("role=") || lower.contains("aria-label") {
        return None;
    }
    // Skip lines that already render an icon-button paired with text content
    // on the same line — heuristic: presence of `>{` followed by a non-empty
    // identifier or string before the next `<`.
    if let Some(after) = line.split_once('>').map(|(_, rest)| rest) {
        let snippet: String = after.chars().take(40).collect();
        let has_text = snippet
            .trim_start()
            .chars()
            .next()
            .map(|c| c.is_alphanumeric() || c == '"' || c == '\'')
            .unwrap_or(false);
        if has_text {
            return None;
        }
    }
    let title = format!("{rel_path}:{line_no}: clickable element missing role/aria-label");
    Some(Finding {
        id: stable_id("clickable-no-label", &format!("{rel_path}|{line_no}")),
        domain: Domain::Other("ux".into()),
        severity: Severity::Medium,
        title: truncate(&title, 120),
        evidence: vec![
            format!("{rel_path}:{line_no}: {}", line.trim()),
            "rule: a11y/clickable-needs-name".into(),
            "Use <button>, or add role=\"button\" + aria-label + key handler".into(),
        ],
        suggested_action: SuggestedAction {
            kind: ActionKind::AmendWorkplan,
            draft_ref: None,
        },
    })
}

fn detect_arbitrary_tailwind_color(rel_path: &str, line_no: u64, line: &str) -> Option<Finding> {
    // `bg-[#abc]`, `text-[#abc]`, `border-[#abc]`, `from-[#abc]`, `to-[#abc]`,
    // `via-[#abc]`, `fill-[#abc]`, `stroke-[#abc]`, `ring-[#abc]`,
    // `shadow-[#abc]` — Tailwind arbitrary value escape hatches that bypass
    // the token palette.
    const PREFIXES: &[&str] = &[
        "bg-[#", "text-[#", "border-[#", "from-[#", "to-[#", "via-[#",
        "fill-[#", "stroke-[#", "ring-[#", "shadow-[#",
    ];
    let hit = PREFIXES.iter().find(|p| line.contains(*p))?;
    let title = format!("{rel_path}:{line_no}: arbitrary Tailwind color {hit}…]");
    Some(Finding {
        id: stable_id("tw-arbitrary-color", &format!("{rel_path}|{line_no}|{hit}")),
        domain: Domain::Other("ux".into()),
        severity: Severity::Low,
        title: truncate(&title, 120),
        evidence: vec![
            format!("{rel_path}:{line_no}: {}", line.trim()),
            format!("rule: design-tokens/no-arbitrary-color (prefix={hit})"),
            "Use a token from tailwind.config (theme.extend.colors) instead".into(),
        ],
        suggested_action: SuggestedAction {
            kind: ActionKind::Informational,
            draft_ref: None,
        },
    })
}

fn detect_inline_style_drift(rel_path: &str, line_no: u64, line: &str) -> Option<Finding> {
    // Inline `style={{ color: '…' }}` / `style={{ background: '…' }}` /
    // `style={{ fontFamily: '…' }}` / `style={{ fontSize: '…' }}` —
    // anything that hard-codes a visual token in a JSX prop.
    let lower = line.to_ascii_lowercase();
    if !lower.contains("style=") {
        return None;
    }
    let needles = [
        "color:", "background:", "background-color:", "fontfamily:",
        "font-family:", "fontsize:", "font-size:", "fontweight:", "font-weight:",
    ];
    let hit = needles.into_iter().find(|n| lower.contains(n))?;
    let title = format!("{rel_path}:{line_no}: inline style drifts from tokens ({hit})");
    Some(Finding {
        id: stable_id("inline-style", &format!("{rel_path}|{line_no}|{hit}")),
        domain: Domain::Other("ux".into()),
        severity: Severity::Low,
        title: truncate(&title, 120),
        evidence: vec![
            format!("{rel_path}:{line_no}: {}", line.trim()),
            format!("rule: design-tokens/no-inline-style (key={hit})"),
            "Move to a Tailwind class or a CSS variable defined in dashboard.css".into(),
        ],
        suggested_action: SuggestedAction {
            kind: ActionKind::Informational,
            draft_ref: None,
        },
    })
}

fn detect_css_color_literal(rel_path: &str, line_no: u64, line: &str) -> Option<Finding> {
    // CSS color drift: hex / rgb() / rgba() / hsl() literals appearing
    // outside the `:root` block where design tokens live. We approximate
    // "outside :root" by skipping lines that look like CSS variable
    // *definitions* (`--token: #abc;`); the rest are usages that should
    // reference `var(--token)` instead.
    let trimmed = line.trim();
    if trimmed.starts_with("--") && trimmed.contains(':') {
        return None;
    }
    let has_hex = scan_hex_color(trimmed).is_some();
    let has_func = ["rgb(", "rgba(", "hsl(", "hsla("]
        .iter()
        .any(|f| trimmed.contains(f));
    if !has_hex && !has_func {
        return None;
    }
    let title = format!("{rel_path}:{line_no}: CSS color literal — should use var(--token)");
    Some(Finding {
        id: stable_id("css-color-literal", &format!("{rel_path}|{line_no}")),
        domain: Domain::Other("ux".into()),
        severity: Severity::Low,
        title: truncate(&title, 120),
        evidence: vec![
            format!("{rel_path}:{line_no}: {}", trimmed),
            "rule: design-tokens/no-color-literal".into(),
            "Define the color in :root as --token-… and reference it here".into(),
        ],
        suggested_action: SuggestedAction {
            kind: ActionKind::Informational,
            draft_ref: None,
        },
    })
}

fn detect_css_font_drift(rel_path: &str, line_no: u64, line: &str) -> Option<Finding> {
    let trimmed = line.trim();
    if !trimmed.starts_with("font-family") && !trimmed.contains(" font-family") {
        return None;
    }
    // Token-defining lines (`--font-…: …`) are fine.
    if trimmed.starts_with("--") {
        return None;
    }
    // `font-family: var(--font-…)` is also fine — it's *using* a token.
    if trimmed.contains("var(--") {
        return None;
    }
    let title = format!("{rel_path}:{line_no}: font-family drift — should use var(--font-token)");
    Some(Finding {
        id: stable_id("css-font-drift", &format!("{rel_path}|{line_no}")),
        domain: Domain::Other("ux".into()),
        severity: Severity::Low,
        title: truncate(&title, 120),
        evidence: vec![
            format!("{rel_path}:{line_no}: {}", trimmed),
            "rule: design-tokens/no-raw-font-family".into(),
        ],
        suggested_action: SuggestedAction {
            kind: ActionKind::Informational,
            draft_ref: None,
        },
    })
}

fn scan_hex_color(s: &str) -> Option<&str> {
    // First `#` followed by exactly 3, 4, 6, or 8 hex chars and a non-hex
    // boundary. Pure-byte scan, no regex dependency.
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j].is_ascii_hexdigit() {
                j += 1;
            }
            let len = j - start;
            let boundary_ok = j == bytes.len() || !bytes[j].is_ascii_alphanumeric();
            if matches!(len, 3 | 4 | 6 | 8) && boundary_ok {
                return Some(&s[i..j]);
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn stable_id(prefix: &str, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prefix.as_bytes());
    hasher.update([0u8]);
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    let head = u64::from_be_bytes(digest[..8].try_into().unwrap());
    format!("f-ux-{prefix}-{head:016x}")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(dir: &Path, rel: &str, body: &str) -> PathBuf {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn missing_assets_dir_is_not_an_error() {
        let tmp = tempdir().unwrap();
        let findings = analyze_ux(tmp.path()).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn dangerous_innerhtml_assignment_is_high_finding() {
        let src = "el.innerHTML = userInput;";
        let f = parse_findings_from_source("src/ui/foo.tsx", src);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::High);
        assert_eq!(f[0].domain, Domain::Other("ux".into()));
        assert_eq!(f[0].suggested_action.kind, ActionKind::DraftWorkplan);
        assert!(f[0].title.contains("innerHTML"));
        assert!(f[0].id.starts_with("f-ux-dangerous-html-"));
    }

    #[test]
    fn dangerous_html_in_css_files_is_not_flagged() {
        let f = parse_findings_from_source("src/dashboard.css", "/* innerHTML notes */");
        assert!(f.is_empty());
    }

    #[test]
    fn img_without_alt_is_medium_finding() {
        let src = r#"<img src="/logo.svg" class="h-8" />"#;
        let f = parse_findings_from_source("src/components/Header.tsx", src);
        assert_eq!(f.len(), 1, "got {f:?}");
        assert_eq!(f[0].severity, Severity::Medium);
        assert!(f[0].title.contains("alt="));
        assert_eq!(f[0].suggested_action.kind, ActionKind::AmendWorkplan);
    }

    #[test]
    fn img_with_alt_passes() {
        let src = r#"<img src="/logo.svg" alt="hex logo" />"#;
        let f = parse_findings_from_source("src/components/Header.tsx", src);
        assert!(f.is_empty(), "expected no findings, got {f:?}");
    }

    #[test]
    fn clickable_div_without_label_is_medium_finding() {
        let src = "<div onClick={handle} class=\"cursor-pointer\"></div>";
        let f = parse_findings_from_source("src/components/Card.tsx", src);
        assert_eq!(f.len(), 1, "got {f:?}");
        assert_eq!(f[0].severity, Severity::Medium);
        assert!(f[0].title.contains("clickable"));
    }

    #[test]
    fn clickable_div_with_aria_label_passes() {
        let src = r#"<div onClick={handle} aria-label="open menu"></div>"#;
        let f = parse_findings_from_source("src/components/Card.tsx", src);
        assert!(f.is_empty(), "got {f:?}");
    }

    #[test]
    fn clickable_div_with_visible_text_passes() {
        // Heuristic: text immediately after the opening tag closer counts as
        // an accessible name on the same line.
        let src = r#"<div onClick={handle}>Open settings</div>"#;
        let f = parse_findings_from_source("src/components/Card.tsx", src);
        assert!(f.is_empty(), "got {f:?}");
    }

    #[test]
    fn arbitrary_tailwind_color_is_low_finding() {
        let src = r#"<button class="bg-[#10b981] text-white">Go</button>"#;
        let f = parse_findings_from_source("src/components/Btn.tsx", src);
        assert_eq!(f.len(), 1, "got {f:?}");
        assert_eq!(f[0].severity, Severity::Low);
        assert_eq!(f[0].suggested_action.kind, ActionKind::Informational);
        assert!(f[0].title.contains("bg-[#"));
    }

    #[test]
    fn inline_style_color_drift_is_low_finding() {
        let src = r#"<span style={{ color: '#fff', fontWeight: 600 }}>x</span>"#;
        let f = parse_findings_from_source("src/components/X.tsx", src);
        // Both `color:` and `font-weight:`/`fontweight:` match, but they share
        // a single line — we want at least the color hit, but dedupe lets up
        // to two distinct rule hits stand. Check the color one is present.
        assert!(
            f.iter().any(|finding| finding.title.contains("color:")),
            "no color finding in {f:?}"
        );
        for finding in &f {
            assert_eq!(finding.severity, Severity::Low);
        }
    }

    #[test]
    fn css_hex_literal_outside_root_is_low_finding() {
        let src = ".panel { background: #1f2937; }";
        let f = parse_findings_from_source("src/dashboard.css", src);
        assert_eq!(f.len(), 1, "got {f:?}");
        assert_eq!(f[0].severity, Severity::Low);
        assert!(f[0].title.contains("CSS color literal"));
    }

    #[test]
    fn css_token_definition_does_not_self_flag() {
        let src = "  --color-bg: #1f2937;";
        let f = parse_findings_from_source("src/dashboard.css", src);
        assert!(f.is_empty(), "token definition should not flag itself: {f:?}");
    }

    #[test]
    fn css_var_reference_does_not_flag_color_or_font() {
        let src = ".x { background: var(--color-bg); font-family: var(--font-mono); }";
        let f = parse_findings_from_source("src/dashboard.css", src);
        assert!(f.is_empty(), "got {f:?}");
    }

    #[test]
    fn css_font_family_with_raw_value_is_low_finding() {
        let src = ".log { font-family: 'Fira Code', monospace; }";
        let f = parse_findings_from_source("src/dashboard.css", src);
        assert_eq!(f.len(), 1, "got {f:?}");
        assert_eq!(f[0].severity, Severity::Low);
        assert!(f[0].title.contains("font-family"));
    }

    #[test]
    fn comment_lines_are_skipped() {
        let src = "// el.innerHTML = bad;\n/* innerHTML in a block comment */";
        let f = parse_findings_from_source("src/x.tsx", src);
        assert!(f.is_empty(), "got {f:?}");
    }

    #[test]
    fn finding_ids_are_stable_and_distinct_by_location() {
        let src = "el.innerHTML = a;\nother.innerHTML = b;";
        let first = parse_findings_from_source("src/x.tsx", src);
        let second = parse_findings_from_source("src/x.tsx", src);
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].id, second[0].id);
        assert_eq!(first[1].id, second[1].id);
        assert_ne!(first[0].id, first[1].id);
    }

    #[test]
    fn analyze_ux_walks_assets_src_and_dedupes() {
        let tmp = tempdir().unwrap();
        let assets = tmp.path();
        write(assets, "src/components/A.tsx", "el.innerHTML = a;\n");
        write(assets, "src/components/B.tsx", "<img src=\"/x.png\" />\n");
        write(assets, "src/dashboard.css", ".x { color: #abcdef; }\n");
        // Should be skipped:
        write(assets, "src/__tests__/A.test.tsx", "el.innerHTML = test;\n");
        write(assets, "src/node_modules/dep/index.tsx", "el.innerHTML = dep;\n");

        let findings = analyze_ux(assets).unwrap();
        // 1 dangerous-html (from A.tsx), 1 img-no-alt (from B.tsx), 1 css color literal (dashboard.css)
        assert_eq!(findings.len(), 3, "got {findings:?}");

        // Severity-desc ordering for downstream synthesis: High first.
        let severities: Vec<Severity> = findings.iter().map(|f| f.severity).collect();
        assert_eq!(severities[0], Severity::High);
        assert!(severities.windows(2).all(|w| w[0] >= w[1]));

        let ux = Domain::Other("ux".into());
        assert!(findings.iter().all(|f| f.domain == ux));
    }

    #[test]
    fn emitted_finding_round_trips_through_yaml() {
        let f = parse_findings_from_source("src/x.tsx", "el.innerHTML = a;")
            .pop()
            .unwrap();
        let yaml = serde_yaml::to_string(&f).expect("serialize yaml");
        let back: Finding = serde_yaml::from_str(&yaml).expect("deserialize yaml");
        assert_eq!(f, back);
        assert!(yaml.contains("domain: ux"), "yaml = {yaml}");
        assert!(yaml.contains("severity: high"), "yaml = {yaml}");
    }
}
