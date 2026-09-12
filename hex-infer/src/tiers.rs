//! Which model serves which kind of work.
//!
//! `.hex/project.json` has declared `inference.tier_models` since
//! ADR-2026-04-12-0202, and nothing read it. The daemon's router did the tier
//! lookup; when the daemon went, the block became decoration and callers
//! hardcoded a model id instead — `hex hey` asked for `gemma4:latest` by name
//! in two places.
//!
//! That is the founding-goal G1 failure in miniature: the test for G1 is that
//! **zero non-test files outside this crate mention a specific provider or
//! model**. A caller that names a model cannot be re-pointed by editing
//! configuration.
//!
//! ```json
//! { "inference": { "tier_models": { "t1": "…", "t2": "…", "t2.5": "…" } } }
//! ```
//!
//! | Tier | For |
//! |---|---|
//! | `t1` | scaffolding, transforms, classification — cheapest that works |
//! | `t2` | ordinary code generation |
//! | `t2.5` | cross-file reasoning, design |
//! | `t3` | frontier work; handled by the `claude` path, not here |

use std::path::{Path, PathBuf};

/// The model configured for `tier`, e.g. `"t1"`.
///
/// `None` when the project declares no model for it. Callers decide what to
/// do with that — there is no default model here on purpose, because guessing
/// one would put a model name back in the code this module exists to remove.
pub fn tier_model(tier: &str) -> Option<String> {
    tier_model_in(&project_root(), tier)
}

/// [`tier_model`], rooted at an explicit directory. Tests drive this.
pub fn tier_model_in(root: &Path, tier: &str) -> Option<String> {
    let text = std::fs::read_to_string(root.join(".hex").join("project.json")).ok()?;
    let root: serde_json::Value = serde_json::from_str(&text).ok()?;
    let models = root.get("inference")?.get("tier_models")?;
    // `t2.5` is a key with a dot in it, so a JSON pointer would misread it.
    let value = models.get(tier).and_then(|v| v.as_str())?;
    (!value.is_empty()).then(|| value.to_string())
}

/// The models the ReAct loop may use, in preference order.
pub fn react_models() -> Vec<String> {
    react_models_in(&project_root())
}

/// [`react_models`], rooted at an explicit directory.
pub fn react_models_in(root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join(".hex").join("project.json")) else {
        return Vec::new();
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    root.get("inference")
        .and_then(|i| i.get("react_models"))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// The repository this process is working in.
fn project_root() -> PathBuf {
    if let Ok(p) = std::env::var("HEX_PROJECT_ROOT") {
        return PathBuf::from(p);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut dir = cwd.as_path();
    loop {
        if dir.join(".hex").is_dir() || dir.join(".git").exists() {
            return dir.to_path_buf();
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => return cwd,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(json: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".hex")).expect("mkdir");
        std::fs::write(dir.path().join(".hex").join("project.json"), json).expect("write");
        dir
    }

    #[test]
    fn a_declared_tier_resolves() {
        let d = project(
            r#"{"inference":{"tier_models":{"t1":"small","t2":"medium","t2.5":"large"}}}"#,
        );
        assert_eq!(tier_model_in(d.path(), "t1").as_deref(), Some("small"));
        assert_eq!(tier_model_in(d.path(), "t2").as_deref(), Some("medium"));
    }

    #[test]
    fn the_dotted_tier_resolves_too() {
        // `t2.5` would be read as a nested path by a JSON pointer.
        let d = project(r#"{"inference":{"tier_models":{"t2.5":"reasoner"}}}"#);
        assert_eq!(tier_model_in(d.path(), "t2.5").as_deref(), Some("reasoner"));
    }

    #[test]
    fn an_undeclared_tier_is_none_not_a_guess() {
        let d = project(r#"{"inference":{"tier_models":{"t1":"small"}}}"#);
        assert_eq!(tier_model_in(d.path(), "t3"), None);
        assert_eq!(tier_model_in(d.path(), ""), None);
    }

    #[test]
    fn an_empty_value_is_not_a_model() {
        let d = project(r#"{"inference":{"tier_models":{"t1":""}}}"#);
        assert_eq!(tier_model_in(d.path(), "t1"), None);
    }

    #[test]
    fn a_missing_or_broken_project_file_is_none_not_an_error() {
        let empty = tempfile::tempdir().expect("tempdir");
        assert_eq!(tier_model_in(empty.path(), "t1"), None);
        assert!(react_models_in(empty.path()).is_empty());

        let broken = project("{ not json");
        assert_eq!(tier_model_in(broken.path(), "t1"), None);

        let no_block = project(r#"{"other": true}"#);
        assert_eq!(tier_model_in(no_block.path(), "t1"), None);
    }

    #[test]
    fn react_models_keep_their_declared_order() {
        let d = project(r#"{"inference":{"react_models":["first","second"]}}"#);
        assert_eq!(react_models_in(d.path()), ["first", "second"]);
    }
}
