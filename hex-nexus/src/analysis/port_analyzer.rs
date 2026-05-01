use serde::Serialize;

#[derive(Serialize)]
struct PortAnalysisReport {
    ports: Vec<String>,
    issues: Vec<String>,
}

pub fn analyze_ports(ports: Vec<&str>) -> PortAnalysisReport {
    let mut issues = Vec::new();

    for port in &ports {
        if port.contains("u32") || port.contains("i32") || port.contains("String") {
            issues.push(format!("Concrete type usage detected in port: {}", port));
        }
    }

    PortAnalysisReport {
        ports: ports.into_iter().map(String::from).collect(),
        issues,
    }
}