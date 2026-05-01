use std::collections::HashMap;
use std::path::Path;
use syn::{File, Item, ItemMod};
use walkdir::WalkDir;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct AdapterAnalysisReport {
    pub cross_adapter_imports: Vec<String>,
    pub coupling_violations: Vec<String>,
}

pub fn analyze_adapters(root_path: &str) -> AdapterAnalysisReport {
    let mut report = AdapterAnalysisReport {
        cross_adapter_imports: Vec::new(),
        coupling_violations: Vec::new(),
    };

    let mut adapter_modules = HashMap::new();

    for entry in WalkDir::new(root_path).into_iter().filter_map(|e| e.ok()) {
        if entry.path().extension().map_or(false, |ext| ext == "rs") {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                if let Ok(ast) = syn::parse_str::<File>(&content) {
                    for item in ast.items {
                        if let Item::Mod(item_mod) = item {
                            if let Some(adapter_name) = extract_adapter_name(entry.path()) {
                                adapter_modules.insert(adapter_name, item_mod);
                            }
                        }
                    }
                }
            }
        }
    }

    for (adapter_name, item_mod) in &adapter_modules {
        if let Some((_, items)) = &item_mod.content {
            for item in items {
                if let Item::Use(item_use) = item {
                    let path = &item_use.path;
                    if path.segments.len() > 1 {
                        let imported_module = path.segments[0].ident.to_string();
                        if adapter_name != &imported_module && adapter_modules.contains_key(&imported_module) {
                            report.cross_adapter_imports.push(format!("{} imports from {}", adapter_name, imported_module));
                        }
                    }
                }
            }
        }
    }

    report
}

fn extract_adapter_name(path: &Path) -> Option<String> {
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|name| name.to_str())
        .map(|s| s.to_string())
}