use serde::Serialize;

#[derive(Serialize)]
struct PortAnalysisReport {
    ports: Vec<String>,
    issues: Vec<String>,
}

pub fn analyze_ports(ports: Vec<&str>) -> Result<PortAnalysisReport, String> {
    let mut issues = Vec::new();
    for port in &ports {
        if port.contains("::") {
            issues.push(format!("Concrete type detected in port: {}", port));
        }
    }

    Ok(PortAnalysisReport {
        ports: ports.into_iter().map(|p| p.to_string()).collect(),
        issues,
    })
}