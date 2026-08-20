//! Lightweight Markdown extraction.
//!
//! No tree-sitter dependency for docs: we scan ATX headings (`#`..`######`) as
//! `DocConcept` candidates and collect inline `[text](link)` reference targets.
//! Prose chunks are returned so the (optional) semantic pass can infer edges.

/// A heading lifted from a doc file.
#[derive(Debug, Clone)]
pub struct DocHeading {
    pub title: String,
    pub level: usize,
    pub line: usize,
}

/// Output of scanning one Markdown/text file.
#[derive(Debug, Clone, Default)]
pub struct DocExtract {
    pub headings: Vec<DocHeading>,
    /// Link targets found in `[text](target)` spans.
    pub links: Vec<String>,
    /// Concatenated prose (headings stripped), for semantic inference.
    pub prose: String,
}

/// True for files the markdown extractor handles.
pub fn is_doc_file(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".md") || p.ends_with(".markdown") || p.ends_with(".mdx") || p.ends_with(".rst")
}

pub fn extract_doc(source: &str) -> DocExtract {
    let mut out = DocExtract::default();
    let mut prose = String::new();
    let mut in_fence = false;

    for (idx, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim_end();
        let trimmed = line.trim_start();

        // Skip fenced code blocks entirely.
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }

        if let Some((level, title)) = parse_heading(trimmed) {
            if !title.is_empty() {
                out.headings.push(DocHeading {
                    title: title.to_string(),
                    level,
                    line: idx + 1,
                });
            }
            continue;
        }

        collect_links(line, &mut out.links);
        if !trimmed.is_empty() {
            prose.push_str(line);
            prose.push('\n');
        }
    }

    out.prose = prose;
    out
}

fn parse_heading(line: &str) -> Option<(usize, &str)> {
    if !line.starts_with('#') {
        return None;
    }
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = line[hashes..].trim();
    // ATX headings require a space after the hashes.
    if line.as_bytes().get(hashes) != Some(&b' ') {
        return None;
    }
    Some((hashes, rest.trim_end_matches('#').trim()))
}

fn collect_links(line: &str, links: &mut Vec<String>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b']' && bytes.get(i + 1) == Some(&b'(') {
            if let Some(end) = line[i + 2..].find(')') {
                let target = line[i + 2..i + 2 + end].trim();
                // Strip an optional title: [t](url "title")
                let target = target.split_whitespace().next().unwrap_or("");
                if !target.is_empty() {
                    links.push(target.to_string());
                }
                i = i + 2 + end + 1;
                continue;
            }
        }
        i += 1;
    }
}
