use std::collections::HashMap;
use std::path::Path;
use syn::{File, ItemFn};
use walkdir::WalkDir;

#[derive(Debug, serde::Serialize)]
pub struct DomainAnalysisReport {
    pub pure_functions: Vec<String>,
    pub functions_with_side_effects: Vec<String>,
}

pub fn analyze_domain(domain_path: &str) -> DomainAnalysisReport {
    let mut report = DomainAnalysisReport {
        pure_functions: Vec::new(),
        functions_with_side_effects: Vec::new(),
    };

    for entry in WalkDir::new(domain_path) {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_file() && path.extension().map_or(false, |ext| ext == "rs") {
            analyze_file(path, &mut report);
        }
    }

    report
}

fn analyze_file(path: &Path, report: &mut DomainAnalysisReport) {
    let content = std::fs::read_to_string(path).unwrap();
    let ast = syn::parse_str::<File>(&content).unwrap();

    for item in ast.items {
        if let syn::Item::Fn(item_fn) = item {
            let function_name = item_fn.sig.ident.to_string();
            if is_pure_function(&item_fn) {
                report.pure_functions.push(function_name);
            } else {
                report.functions_with_side_effects.push(function_name);
            }
        }
    }
}

fn is_pure_function(item_fn: &ItemFn) -> bool {
    // Basic check - looking for side-effect indicators
    let mut is_pure = true;
    for stmt in &item_fn.block.stmts {
        if let syn::Stmt::Expr(expr) = stmt {
            if let syn::Expr::Call(_) = expr {
                is_pure = false;
                break;
            }
        }
    }
    is_pure
}