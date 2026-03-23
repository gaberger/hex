//! Prompt template engine for hex dev TUI (ADR-2603232005).
//!
//! Loads prompt templates from embedded assets (`hex-cli/assets/prompts/`)
//! and renders them by replacing `{{placeholder}}` tokens with provided values.
//!
//! Templates are baked into the binary at compile time via `rust-embed` (see `assets.rs`).

use std::collections::HashMap;
use tracing::warn;

use crate::assets::Assets;

/// Prefix within the embedded asset bundle where prompt templates live.
const PROMPTS_PREFIX: &str = "prompts/";

/// A loaded prompt template with placeholder expansion.
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    /// Template name (e.g. "adr-generate")
    pub name: String,
    /// Raw template content with `{{placeholder}}` tokens
    raw: String,
}

impl PromptTemplate {
    /// Load a prompt template by name (without path prefix or `.md` extension).
    ///
    /// # Examples
    /// ```no_run
    /// let tmpl = PromptTemplate::load("adr-generate").unwrap();
    /// ```
    pub fn load(name: &str) -> anyhow::Result<Self> {
        let asset_path = format!("{}{}.md", PROMPTS_PREFIX, name);
        let raw = Assets::get_str(&asset_path).ok_or_else(|| {
            anyhow::anyhow!(
                "Prompt template '{}' not found (looked for '{}'). Available: {:?}",
                name,
                asset_path,
                Self::list()
            )
        })?;
        Ok(Self {
            name: name.to_string(),
            raw,
        })
    }

    /// Render the template by replacing all `{{key}}` placeholders with values
    /// from the provided context map.
    ///
    /// - Known placeholders present in `context` are replaced with their value.
    /// - Known placeholders absent from `context` are replaced with an empty string
    ///   and a warning is logged.
    /// - Literal `{{` sequences that don't match a placeholder pattern are left as-is.
    pub fn render(&self, context: &HashMap<String, String>) -> String {
        let mut output = self.raw.clone();

        // Find all {{placeholder}} tokens in the template
        let placeholders = Self::extract_placeholders(&self.raw);

        for placeholder in &placeholders {
            let token = format!("{{{{{}}}}}", placeholder);
            if let Some(value) = context.get(placeholder.as_str()) {
                output = output.replace(&token, value);
            } else {
                warn!(
                    template = %self.name,
                    placeholder = %placeholder,
                    "Missing placeholder value — replacing with empty string"
                );
                output = output.replace(&token, "");
            }
        }

        output
    }

    /// Convenience: render with a slice of `(key, value)` pairs.
    pub fn render_pairs(&self, pairs: &[(&str, &str)]) -> String {
        let context: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self.render(&context)
    }

    /// List all available prompt template names.
    pub fn list() -> Vec<String> {
        Assets::iter()
            .filter_map(|path| {
                let path_str = path.as_ref();
                if let Some(rest) = path_str.strip_prefix(PROMPTS_PREFIX) {
                    rest.strip_suffix(".md").map(|name| name.to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Extract all `{{placeholder}}` names from a template string.
    fn extract_placeholders(content: &str) -> Vec<String> {
        let mut placeholders = Vec::new();
        let mut rest = content;

        while let Some(start) = rest.find("{{") {
            let after_open = &rest[start + 2..];
            if let Some(end) = after_open.find("}}") {
                let name = after_open[..end].trim();
                // Only treat as a placeholder if it looks like an identifier
                // (alphanumeric + underscores, no spaces or special chars)
                if !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_')
                {
                    if !placeholders.contains(&name.to_string()) {
                        placeholders.push(name.to_string());
                    }
                }
                rest = &after_open[end + 2..];
            } else {
                break;
            }
        }

        placeholders
    }

    /// Get the raw template content (before rendering).
    pub fn raw_content(&self) -> &str {
        &self.raw
    }

    /// Get the list of placeholder names found in this template.
    pub fn placeholders(&self) -> Vec<String> {
        Self::extract_placeholders(&self.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_adr_generate() {
        let tmpl = PromptTemplate::load("adr-generate");
        assert!(tmpl.is_ok(), "adr-generate template should load");
        let tmpl = tmpl.unwrap();
        assert_eq!(tmpl.name, "adr-generate");
        assert!(tmpl.raw.contains("{{user_description}}"));
    }

    #[test]
    fn load_workplan_generate() {
        let tmpl = PromptTemplate::load("workplan-generate");
        assert!(tmpl.is_ok(), "workplan-generate template should load");
        assert!(tmpl.unwrap().raw.contains("{{adr_content}}"));
    }

    #[test]
    fn load_code_generate() {
        let tmpl = PromptTemplate::load("code-generate");
        assert!(tmpl.is_ok(), "code-generate template should load");
        assert!(tmpl.unwrap().raw.contains("{{target_file}}"));
    }

    #[test]
    fn load_test_generate() {
        let tmpl = PromptTemplate::load("test-generate");
        assert!(tmpl.is_ok(), "test-generate template should load");
        assert!(tmpl.unwrap().raw.contains("{{source_file}}"));
    }

    #[test]
    fn load_fix_violations() {
        let tmpl = PromptTemplate::load("fix-violations");
        assert!(tmpl.is_ok(), "fix-violations template should load");
        assert!(tmpl.unwrap().raw.contains("{{violations}}"));
    }

    #[test]
    fn load_nonexistent_returns_error() {
        let result = PromptTemplate::load("nonexistent-template");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
    }

    #[test]
    fn render_replaces_placeholders() {
        let tmpl = PromptTemplate::load("adr-generate").unwrap();
        let mut ctx = HashMap::new();
        ctx.insert("user_description".to_string(), "Add caching layer".to_string());
        ctx.insert("existing_adrs".to_string(), "ADR-001, ADR-002".to_string());
        ctx.insert("architecture_summary".to_string(), "Rust + TypeScript hex project".to_string());
        ctx.insert("related_adrs".to_string(), "ADR-001".to_string());

        let rendered = tmpl.render(&ctx);
        assert!(rendered.contains("Add caching layer"));
        assert!(rendered.contains("ADR-001, ADR-002"));
        assert!(!rendered.contains("{{user_description}}"));
    }

    #[test]
    fn render_pairs_works() {
        let tmpl = PromptTemplate::load("fix-violations").unwrap();
        let rendered = tmpl.render_pairs(&[
            ("violations", "Cross-adapter import in foo.ts"),
            ("file_content", "import { bar } from '../other-adapter/bar.js';"),
            ("boundary_rules", "Adapters must not import other adapters"),
        ]);
        assert!(rendered.contains("Cross-adapter import"));
        assert!(!rendered.contains("{{violations}}"));
    }

    #[test]
    fn missing_placeholder_becomes_empty() {
        let tmpl = PromptTemplate::load("adr-generate").unwrap();
        let ctx = HashMap::new(); // no values at all
        let rendered = tmpl.render(&ctx);
        // Placeholders should be gone (replaced with empty string)
        assert!(!rendered.contains("{{user_description}}"));
        assert!(!rendered.contains("{{existing_adrs}}"));
    }

    #[test]
    fn list_returns_all_templates() {
        let templates = PromptTemplate::list();
        assert!(templates.contains(&"adr-generate".to_string()));
        assert!(templates.contains(&"workplan-generate".to_string()));
        assert!(templates.contains(&"code-generate".to_string()));
        assert!(templates.contains(&"test-generate".to_string()));
        assert!(templates.contains(&"fix-violations".to_string()));
        assert_eq!(templates.len(), 5);
    }

    #[test]
    fn extract_placeholders_finds_all() {
        let tmpl = PromptTemplate::load("code-generate").unwrap();
        let placeholders = tmpl.placeholders();
        assert!(placeholders.contains(&"step_description".to_string()));
        assert!(placeholders.contains(&"target_file".to_string()));
        assert!(placeholders.contains(&"ast_summary".to_string()));
        assert!(placeholders.contains(&"port_interfaces".to_string()));
        assert!(placeholders.contains(&"boundary_rules".to_string()));
        assert!(placeholders.contains(&"language".to_string()));
    }

    #[test]
    fn markdown_braces_not_treated_as_placeholders() {
        // Template content like ```{{#if mandatory}}``` should not be treated
        // as a placeholder because '#' is not alphanumeric or underscore
        let content = "Hello {{#if foo}}world{{/if}}";
        let placeholders = PromptTemplate::extract_placeholders(content);
        // #if and /if contain non-identifier chars, should be skipped
        assert!(placeholders.is_empty());
    }
}
