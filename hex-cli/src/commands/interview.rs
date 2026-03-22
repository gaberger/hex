//! Project interview for hex init in empty directories (ADR-055).

use anyhow::Result;
use colored::Colorize;
use dialoguer::{Input, Select};
use std::path::Path;

#[derive(Debug, Clone)]
pub enum ProjectLanguage {
    Rust,
    Go,
    TypeScript,
    Python,
    Multi(String),
}

impl std::fmt::Display for ProjectLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rust => write!(f, "Rust"),
            Self::Go => write!(f, "Go"),
            Self::TypeScript => write!(f, "TypeScript"),
            Self::Python => write!(f, "Python"),
            Self::Multi(s) => write!(f, "Multi-language ({})", s),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ProjectType {
    Cli,
    WebService,
    Library,
    FullStack,
    Infrastructure,
    Other(String),
}

impl std::fmt::Display for ProjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cli => write!(f, "CLI tool"),
            Self::WebService => write!(f, "Web service / API"),
            Self::Library => write!(f, "Library / SDK"),
            Self::FullStack => write!(f, "Full-stack application"),
            Self::Infrastructure => write!(f, "Infrastructure / DevOps"),
            Self::Other(s) => write!(f, "{}", s),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub purpose: String,
}

#[derive(Debug, Clone)]
pub struct ProjectInterview {
    pub name: String,
    pub description: String,
    pub language: ProjectLanguage,
    pub project_type: ProjectType,
    pub constraints: Vec<String>,
    pub dependencies: Vec<Dependency>,
}

/// Check if a directory is empty or only has .git/ and/or .hex/ (no source files).
pub fn is_empty_project(path: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else {
        return true;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Allow .git, .hex, .gitignore, .gitattributes — these don't count as "project content"
        if name_str.starts_with(".git") || name_str == ".hex" {
            continue;
        }
        // Any other file or directory means the project has content
        return false;
    }
    true
}

/// Run the interactive project interview.
pub fn run_interview(default_name: &str) -> Result<ProjectInterview> {
    println!();
    println!(
        "{} {} — New Project Setup",
        "\u{2b21}".cyan(),
        "hex".cyan().bold()
    );
    println!("{}", "\u{2500}".repeat(40).dimmed());
    println!();

    let name: String = Input::new()
        .with_prompt("What is this project called?")
        .default(default_name.to_string())
        .interact_text()?;

    let description: String = Input::new()
        .with_prompt("Describe the project in 1-2 sentences")
        .interact_text()?;

    let lang_options = &["Rust", "Go", "TypeScript", "Python", "Multi-language"];
    let lang_idx = Select::new()
        .with_prompt("What is the primary language?")
        .items(lang_options)
        .default(0)
        .interact()?;

    let language = match lang_idx {
        0 => ProjectLanguage::Rust,
        1 => ProjectLanguage::Go,
        2 => ProjectLanguage::TypeScript,
        3 => ProjectLanguage::Python,
        _ => {
            let detail: String = Input::new()
                .with_prompt("Which languages?")
                .interact_text()?;
            ProjectLanguage::Multi(detail)
        }
    };

    let type_options = &[
        "CLI tool",
        "Web service / API",
        "Library / SDK",
        "Full-stack application",
        "Infrastructure / DevOps",
        "Other",
    ];
    let type_idx = Select::new()
        .with_prompt("What kind of project is this?")
        .items(type_options)
        .default(0)
        .interact()?;

    let project_type = match type_idx {
        0 => ProjectType::Cli,
        1 => ProjectType::WebService,
        2 => ProjectType::Library,
        3 => ProjectType::FullStack,
        4 => ProjectType::Infrastructure,
        _ => {
            let detail: String = Input::new()
                .with_prompt("Describe the project type")
                .interact_text()?;
            ProjectType::Other(detail)
        }
    };

    let constraints_input: String = Input::new()
        .with_prompt("Key constraints or non-negotiables (comma-separated, or empty)")
        .default(String::new())
        .allow_empty(true)
        .interact_text()?;

    let constraints: Vec<String> = constraints_input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let deps_input: String = Input::new()
        .with_prompt("External dependencies (name:purpose pairs, comma-separated, or empty)")
        .default(String::new())
        .allow_empty(true)
        .interact_text()?;

    let dependencies: Vec<Dependency> = deps_input
        .split(',')
        .filter_map(|s| {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            let parts: Vec<&str> = s.splitn(2, ':').collect();
            Some(Dependency {
                name: parts[0].trim().to_string(),
                purpose: parts.get(1).map(|p| p.trim().to_string()).unwrap_or_default(),
            })
        })
        .collect();

    println!();
    println!("{} Interview complete!", "\u{2713}".green());

    Ok(ProjectInterview {
        name,
        description,
        language,
        project_type,
        constraints,
        dependencies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn empty_dir_is_empty_project() {
        let dir = tempfile::tempdir().unwrap();
        assert!(is_empty_project(dir.path()));
    }

    #[test]
    fn git_only_is_empty_project() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        assert!(is_empty_project(dir.path()));
    }

    #[test]
    fn git_and_hex_is_empty_project() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::create_dir(dir.path().join(".hex")).unwrap();
        assert!(is_empty_project(dir.path()));
    }

    #[test]
    fn dir_with_source_is_not_empty() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        assert!(!is_empty_project(dir.path()));
    }

    #[test]
    fn dir_with_subdir_is_not_empty() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        assert!(!is_empty_project(dir.path()));
    }

    #[test]
    fn gitignore_does_not_count_as_content() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        assert!(is_empty_project(dir.path()));
    }
}
