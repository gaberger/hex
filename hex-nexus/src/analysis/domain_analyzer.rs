use std::collections::HashMap;
use std::fs;
use std::path::Path;
use serde::Serialize;

#[derive(Serialize)]
struct FunctionAnalysis {
    name: String,
    is_pure: bool,
    side_effects: Vec<String>,
}

#[derive(Serialize)]
pub struct DomainAnalysisReport {
    domain_path: String,
    functions: Vec<FunctionAnalysis>,
}

pub fn analyze_domain(domain_path: &str) -> DomainAnalysisReport {
    let mut report = DomainAnalysisReport {
        domain_path: domain_path.to_string(),
        functions: Vec::new(),
    };

    if let Ok(entries) = fs::read_dir(domain_path) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(contents) = fs::read_to_string(&path) {
                        let function_analysis = analyze_file_contents(&contents);
                        report.functions.extend(function_analysis);
                    }
                }
            }
        }
    }

    report
}

fn analyze_file_contents(contents: &str) -> Vec<FunctionAnalysis> {
    let mut functions = Vec::new();
    let lines: Vec<&str> = contents.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        if line.trim().starts_with("fn ") {
            let name = extract_function_name(line);
            let (is_pure, side_effects) = check_function_purity(&lines, i);
            functions.push(FunctionAnalysis {
                name,
                is_pure,
                side_effects,
            });
        }
    }

    functions
}

fn extract_function_name(line: &str) -> String {
    line.trim()
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .trim_end_matches('{')
        .trim()
        .to_string()
}

fn check_function_purity(lines: &[&str], start_idx: usize) -> (bool, Vec<String>) {
    let mut is_pure = true;
    let mut side_effects = Vec::new();

    for line in &lines[start_idx..] {
        if line.contains("mut ") || line.contains("unwrap()") || line.contains("expect(") {
            is_pure = false;
            side_effects.push(line.to_string());
        }
        if line.trim().ends_with("}") {
            break;
        }
    }

    (is_pure, side_effects)
}