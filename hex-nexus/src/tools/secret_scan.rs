//! `secret_scan` — regex-based pattern scan for hardcoded secrets/credentials.
//!
//! Used by CISO to audit repo source files for exposed credentials, API keys,
//! private keys, and other sensitive patterns. Returns file:line:snippet matches
//! capped at 200 per call, with pattern classification.

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Instant;
use tokio::fs;

use super::{Tool, ToolResult};

const MAX_MATCHES_DEFAULT: usize = 50;
const MAX_MATCHES_HARD_CAP: usize = 200;
const MAX_FILES_DEFAULT: usize = 1000;
const MAX_OUTPUT_BYTES: usize = 16 * 1024;

pub struct SecretScan;

struct SecretPattern {
    name: &'static str,
    regex: Regex,
}

impl SecretPattern {
    fn new(name: &'static str, pattern: &str) -> Self {
        Self {
            name,
            regex: Regex::new(pattern).expect("valid regex"),
        }
    }
}

fn build_patterns() -> Vec<SecretPattern> {
    vec![
        SecretPattern::new("aws_access_key", r"AKIA[0-9A-Z]{16}"),
        SecretPattern::new(
            "rsa_private_key",
            r"-----BEGIN.*PRIVATE KEY-----",
        ),
        SecretPattern::new(
            "generic_secret",
            r#"(?i)(api[_-]?key|password|secret|token)\s*[=:]\s*["'][^"']{8,}["']"#,
        ),
        SecretPattern::new(
            "jwt",
            r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
        ),
        SecretPattern::new(
            "bearer_token",
            r"(?i)bearer\s+[a-zA-Z0-9_\-\.]{20,}",
        ),
        SecretPattern::new(
            "github_token",
            r"gh[pousr]_[A-Za-z0-9_]{36,}",
        ),
        SecretPattern::new(
            "slack_token",
            r"xox[baprs]-[0-9]{10,13}-[0-9]{10,13}-[a-zA-Z0-9]{24,}",
        ),
    ]
}

#[async_trait]
impl Tool for SecretScan {
    fn name(&self) -> &'static str {
        "secret_scan"
    }
    fn description(&self) -> &'static str {
        "Regex-based pattern scan for hardcoded secrets/credentials in repo \
         source files. Detects AWS keys (AKIA*), RSA private keys, generic \
         API_KEY/password/secret assignments, JWT tokens, bearer tokens, \
         GitHub/Slack tokens. Returns file:line:snippet matches capped at \
         200 per call. Used by CISO for security audit."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Repo-relative paths to scan (files or directories). Required.",
                },
                "max_files": {
                    "type": "integer",
                    "description": "Max files to scan. Default 1000.",
                }
            },
            "required": ["paths"]
        })
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let paths_arr = match input.get("paths").and_then(|v| v.as_array()) {
            Some(arr) if !arr.is_empty() => arr,
            _ => return ToolResult::err("missing or empty `paths` array", start.elapsed().as_millis() as u64),
        };

        let paths: Vec<String> = paths_arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        if paths.is_empty() {
            return ToolResult::err("no valid paths in `paths` array", start.elapsed().as_millis() as u64);
        }

        let max_files = input
            .get("max_files")
            .and_then(|v| v.as_u64())
            .unwrap_or(MAX_FILES_DEFAULT as u64) as usize;

        let repo_root = std::env::var("HEX_REPO_ROOT")
            .unwrap_or_else(|_| "/home/gary/hex-intf".to_string());
        let repo_path = PathBuf::from(&repo_root);

        let patterns = build_patterns();
        let mut matches: Vec<Value> = Vec::new();
        let mut files_scanned = 0usize;
        let mut truncated = false;
        let mut output_bytes = 0usize;

        for rel_path in &paths {
            let abs_path = repo_path.join(rel_path);
            if let Err(e) = scan_path(
                &abs_path,
                rel_path,
                &patterns,
                &mut matches,
                &mut files_scanned,
                &mut output_bytes,
                max_files,
                MAX_MATCHES_HARD_CAP,
                MAX_OUTPUT_BYTES,
                &mut truncated,
            )
            .await
            {
                // Non-fatal: log error and continue
                eprintln!("secret_scan: scan_path error for {}: {}", rel_path, e);
            }
            if truncated {
                break;
            }
        }

        let elapsed = start.elapsed().as_millis() as u64;
        let result = json!({
            "matches": matches,
            "total_matches": matches.len(),
            "files_scanned": files_scanned,
            "truncated": truncated,
        });

        if truncated {
            ToolResult::ok_truncated(result, elapsed)
        } else {
            ToolResult::ok(result, elapsed)
        }
    }
}

// Operator surgical repair: async recursion requires boxing — wrapping the
// recursive call in Box::pin to give the future a known size. The persona's
// autonomous emit missed this; next-layer homeostasis is the cargo_check
// post-patch verifier hook.
#[allow(clippy::too_many_arguments)]
fn scan_path<'a>(
    abs_path: &'a PathBuf,
    rel_path: &'a str,
    patterns: &'a [SecretPattern],
    matches: &'a mut Vec<Value>,
    files_scanned: &'a mut usize,
    output_bytes: &'a mut usize,
    max_files: usize,
    max_matches: usize,
    max_output_bytes: usize,
    truncated: &'a mut bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Box<dyn std::error::Error>>> + Send + 'a>> {
    Box::pin(async move {
    if *files_scanned >= max_files || matches.len() >= max_matches || *output_bytes >= max_output_bytes {
        *truncated = true;
        return Ok(());
    }

    let metadata = fs::metadata(abs_path).await?;
    if metadata.is_dir() {
        let mut entries = fs::read_dir(abs_path).await?;
        while let Some(entry) = entries.next_entry().await? {
            let child_path = entry.path();
            let child_rel = format!("{}/{}", rel_path.trim_end_matches('/'), entry.file_name().to_string_lossy());
            scan_path(
                &child_path,
                &child_rel,
                patterns,
                matches,
                files_scanned,
                output_bytes,
                max_files,
                max_matches,
                max_output_bytes,
                truncated,
            )
            .await?;
            if *truncated {
                break;
            }
        }
    } else if metadata.is_file() {
        // Skip large files
        if metadata.len() > 512 * 1024 {
            return Ok(());
        }
        // Skip binary files by extension heuristic
        if let Some(ext) = abs_path.extension() {
            let ext_str = ext.to_string_lossy().to_lowercase();
            if matches!(ext_str.as_str(), "png" | "jpg" | "jpeg" | "gif" | "ico" | "woff" | "woff2" | "ttf" | "eot" | "mp4" | "webm" | "zip" | "tar" | "gz" | "so" | "dylib" | "dll" | "exe" | "bin") {
                return Ok(());
            }
        }

        let content = fs::read_to_string(abs_path).await?;
        *files_scanned += 1;

        for (line_idx, line) in content.lines().enumerate() {
            if matches.len() >= max_matches || *output_bytes >= max_output_bytes {
                *truncated = true;
                return Ok(());
            }
            for pattern in patterns {
                if pattern.regex.is_match(line) {
                    let snippet = line.chars().take(200).collect::<String>();
                    let match_json = json!({
                        "path": rel_path,
                        "line": line_idx + 1,
                        "pattern_name": pattern.name,
                        "snippet": snippet,
                    });
                    let match_bytes = serde_json::to_string(&match_json).unwrap_or_default().len();
                    *output_bytes += match_bytes;
                    matches.push(match_json);
                    if matches.len() >= max_matches || *output_bytes >= max_output_bytes {
                        *truncated = true;
                        return Ok(());
                    }
                }
            }
        }
    }
    Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_requires_paths() {
        let s = SecretScan.input_schema();
        let req = s.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(req.iter().any(|v| v.as_str() == Some("paths")));
    }

    #[test]
    fn patterns_compile() {
        let patterns = build_patterns();
        assert!(!patterns.is_empty(), "must have at least one pattern");
        for p in &patterns {
            assert!(!p.name.is_empty());
            // Patterns should compile (already asserted in SecretPattern::new)
        }
    }

    #[test]
    fn aws_key_pattern() {
        let patterns = build_patterns();
        let aws = patterns.iter().find(|p| p.name == "aws_access_key").unwrap();
        assert!(aws.regex.is_match("AKIAIOSFODNN7EXAMPLE"));
        assert!(!aws.regex.is_match("NOTANAWSKEY"));
    }

    #[test]
    fn generic_secret_pattern() {
        let patterns = build_patterns();
        let gen = patterns.iter().find(|p| p.name == "generic_secret").unwrap();
        assert!(gen.regex.is_match(r#"api_key="abcd1234efgh5678""#));
        assert!(gen.regex.is_match(r#"PASSWORD = "supersecret123""#));
        assert!(!gen.regex.is_match(r#"api_key=short"#)); // too short
    }
}
