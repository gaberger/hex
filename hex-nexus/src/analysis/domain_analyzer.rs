use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Serialize)]
struct FunctionAnalysis {
    name: String,
    is_pure: bool,
    side_effects: Vec<String>,
}

#[derive(Serialize)]
struct DomainAnalysisReport {
    domain: String,
    functions: Vec<FunctionAnalysis>,
}

pub fn analyze_domain(domain_path: &str) -> Result<DomainAnalysisReport, Box<dyn std::error::Error>> {
    let path = Path::new(domain_path);
    let mut functions = Vec::new();

    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let file_path = entry.path();
            if file_path.is_file() && file_path.extension().and_then(|s| s.to_str()) == Some("rs") {
                let content = fs::read_to_string(&file_path)?;
                let function_analysis = analyze_file(&content);
                functions.extend(function_analysis);
            }
        }
    }

    Ok(DomainAnalysisReport {
        domain: domain_path.to_string(),
        functions,
    })
}

fn analyze_file(content: &str) -> Vec<FunctionAnalysis> {
    let mut functions = Vec::new();
    let mut current_function = None;

    for line in content.lines() {
        if line.trim().starts_with("fn ") {
            let function_name = line.trim().split_whitespace().nth(1).unwrap_or("").to_string();
            current_function = Some(FunctionAnalysis {
                name: function_name.clone(),
                is_pure: true,
                side_effects: Vec::new(),
            });
        }

        if let Some(ref mut func) = current_function {
            if line.contains("mut ") || line.contains("unwrap()") || line.contains("expect(") {
                func.is_pure = false;
                func.side_effects.push(line.trim().to_string());
            }
        }

        if line.trim().ends_with("}") {
            if let Some(func) = current_function.take() {
                functions.push(func);
            }
        }
    }

    functions
}