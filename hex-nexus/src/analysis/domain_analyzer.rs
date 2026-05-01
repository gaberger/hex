use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Serialize)]
pub struct DomainAnalysis {
    pure_functions: Vec<String>,
    side_effects: Vec<String>,
}

pub fn analyze_domain(domain_path: &str) -> DomainAnalysis {
    let mut pure_functions = Vec::new();
    let mut side_effects = Vec::new();

    if let Ok(entries) = fs::read_dir(Path::new(domain_path)) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if content.contains("fn") {
                            if content.contains("mut ") || content.contains("unsafe ") {
                                side_effects.push(path.display().to_string());
                            } else {
                                pure_functions.push(path.display().to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    DomainAnalysis {
        pure_functions,
        side_effects,
    }
}