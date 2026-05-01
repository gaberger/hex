use std::collections::HashMap;
use std::fs;
use std::path::Path;
use syn::{File, ItemFn, visit::Visit};

#[derive(Debug, serde::Serialize)]
pub struct FunctionAnalysis {
    pub name: String,
    pub is_pure: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct DomainAnalysisReport {
    pub domain: String,
    pub functions: Vec<FunctionAnalysis>,
}

struct FunctionVisitor {
    functions: Vec<FunctionAnalysis>,
}

impl FunctionVisitor {
    fn new() -> Self {
        Self {
            functions: Vec::new(),
        }
    }
}

impl<'ast> Visit<'ast> for FunctionVisitor {
    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        let mut is_pure = true;
        
        // Simple heuristic: Functions with side effects often have extern blocks or unsafe
        if node.sig.unsafety.is_some() {
            is_pure = false;
        }
        
        self.functions.push(FunctionAnalysis {
            name: node.sig.ident.to_string(),
            is_pure,
        });
        
        syn::visit::visit_item_fn(self, node);
    }
}

pub fn analyze_domain(domain_path: &str) -> DomainAnalysisReport {
    let mut functions = Vec::new();
    let path = Path::new(domain_path);

    if path.is_dir() {
        for entry in fs::read_dir(path).expect("Failed to read domain directory") {
            let entry = entry.expect("Failed to read directory entry");
            if entry.path().is_file() && entry.path().extension().map_or(false, |ext| ext == "rs") {
                let content = fs::read_to_string(entry.path()).expect("Failed to read file");
                let syntax: File = syn::parse_str(&content).expect("Failed to parse Rust file");
                
                let mut visitor = FunctionVisitor::new();
                visitor.visit_file(&syntax);
                functions.extend(visitor.functions);
            }
        }
    }

    DomainAnalysisReport {
        domain: domain_path.to_string(),
        functions,
    }
}