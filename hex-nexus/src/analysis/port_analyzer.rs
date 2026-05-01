use serde::Serialize;

#[derive(Serialize)]
struct PortAnalysisReport {
    ports: Vec<String>,
    issues: Vec<String>,
}

pub fn analyze_ports(ports: &[String]) -> PortAnalysisReport {
    let mut issues = Vec::new();

    for port in ports {
        if port.contains("ConcreteType") {
            issues.push(format!("Port {} uses a concrete type", port));
        }
    }

    PortAnalysisReport {
        ports: ports.to_vec(),
        issues,
    }
}