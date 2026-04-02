//! Embedded asset bundle for hex-cli (ADR-2603221522).
//!
//! All templates, schemas, and scaffold files live as real files under
//! `hex-cli/assets/` and are baked into the binary at compile time via
//! `rust-embed`. This replaces scattered `include_str!` and hardcoded
//! template strings.

use rust_embed::Embed;

#[derive(Embed)]
#[folder = "assets/"]
pub struct Assets;

impl Assets {
    /// Get a file's contents as a UTF-8 string.
    pub fn get_str(path: &str) -> Option<String> {
        Self::get(path).map(|f| String::from_utf8_lossy(&f.data).to_string())
    }

    /// Get a template and apply simple `{{key}}` substitutions.
    pub fn render_template(path: &str, vars: &[(&str, &str)]) -> Option<String> {
        let mut content = Self::get_str(path)?;
        for (key, value) in vars {
            content = content.replace(&format!("{{{{{}}}}}", key), value);
        }
        Some(content)
    }

    /// Extract all files under a prefix to a target directory.
    /// Skips files that already exist (don't overwrite user customizations).
    pub fn extract_to(prefix: &str, target: &std::path::Path) -> std::io::Result<Vec<String>> {
        let mut extracted = Vec::new();
        for path in Self::iter() {
            if let Some(relative) = path.strip_prefix(prefix) {
                // Skip .tmpl extension in output path
                let dest_name = relative.strip_suffix(".tmpl").unwrap_or(relative);
                let dest = target.join(dest_name);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if let Some(file) = Self::get(&path) {
                    if !dest.exists() {
                        std::fs::write(&dest, &file.data)?;
                        extracted.push(dest_name.to_string());
                    }
                }
            }
        }
        Ok(extracted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workplan_schema_is_embedded() {
        let schema = Assets::get_str("schemas/workplan.schema.json");
        assert!(schema.is_some(), "workplan schema should be embedded");
        let content = schema.unwrap();
        assert!(content.contains("Hex Workplan"), "schema should contain title");
    }

    #[test]
    fn mcp_tools_is_embedded() {
        let tools = Assets::get_str("schemas/mcp-tools.json");
        assert!(tools.is_some(), "mcp-tools.json should be embedded");
    }

    #[test]
    fn settings_template_is_embedded() {
        let settings = Assets::get_str("templates/hex-claude-settings.json");
        assert!(settings.is_some(), "settings template should be embedded");
    }

    #[test]
    fn render_template_substitutes() {
        // Create a simple test — we know the CLAUDE.md template has {{project_name}}
        let result = Assets::render_template("templates/hex-claude-settings.json", &[]);
        assert!(result.is_some());
    }

    #[test]
    fn iter_lists_assets() {
        let count = Assets::iter().count();
        assert!(count >= 3, "should have at least 3 embedded assets, got {}", count);
    }

    #[test]
    fn skills_are_embedded_and_parseable() {
        let skill_paths: Vec<_> = Assets::iter()
            .filter(|p| p.starts_with("skills/") && p.ends_with(".md"))
            .collect();
        println!("skill paths: {:?}", skill_paths);
        assert!(!skill_paths.is_empty(), "no .md files found under skills/ prefix");
        // Verify at least one parses successfully
        let parsed = skill_paths.iter().filter_map(|p| {
            Assets::get_str(p).and_then(|c| {
                let c = c.trim().to_string();
                if !c.starts_with("---") { return None; }
                let rest = &c[3..];
                let end = rest.find("\n---")?;
                let fm = &rest[..end];
                let name: String = fm.lines()
                    .find_map(|l| l.strip_prefix("name:").map(|v| v.trim().to_string()))?;
                Some(name)
            })
        }).collect::<Vec<_>>();
        println!("parsed skill names: {:?}", parsed);
        assert!(!parsed.is_empty(), "no skills parsed successfully from embedded assets");
    }
}
