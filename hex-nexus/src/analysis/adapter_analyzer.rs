use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize)]
pub struct CrossAdapterImport {
    pub source: String,
    pub target: String,
    pub file: String,
    pub line: usize,
}

#[derive(Serialize)]
pub struct AdapterAnalysisReport {
    pub cross_adapter_imports: Vec<CrossAdapterImport>,
    pub coupling_violations: Vec<(String, String)>,
}

pub fn analyze_adapters(module_tree: &HashMap<String, Vec<String>>) -> AdapterAnalysisReport {
    let mut cross_adapter_imports = Vec::new();
    let mut coupling_violations = Vec::new();

    for (source_module, imports) in module_tree {
        for target_module in imports {
            if source_module.starts_with("adapters::") && target_module.starts_with("adapters::") {
                if !source_module.split("::").nth(1).unwrap()
                    .eq(target_module.split("::").nth(1).unwrap())
                {
                    let violation = (
                        source_module.clone(),
                        target_module.clone(),
                    );
                    coupling_violations.push(violation);
                }
            }
        }
    }

    AdapterAnalysisReport {
        cross_adapter_imports,
        coupling_violations,
    }
}