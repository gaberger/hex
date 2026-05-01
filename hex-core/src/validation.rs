// Domain: Critical path validation
// Prevents autonomous agents from modifying control plane infrastructure

/// Files that autonomous agents cannot modify
/// These are critical to daemon operation and system stability
pub const CRITICAL_FILES: &[&str] = &[
    "hex-cli/src/commands/sched.rs",        // Daemon control plane
    "hex-cli/src/commands/monitor.rs",      // Observability
    "hex-cli/src/main.rs",                  // CLI entry point
    "hex-agent/src/workplan_executor.rs",   // Execution engine
    "hex-agent/src/main.rs",                // Agent entry point
    "hex-agent/src/inference_client.rs",    // Inference routing
];

/// Check if a file path is critical infrastructure
/// Returns true if path matches any CRITICAL_FILES pattern
pub fn is_critical_path(path: &str) -> bool {
    CRITICAL_FILES.iter().any(|&critical| {
        path.contains(critical) || path.ends_with(critical)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_critical_path_detection() {
        assert!(is_critical_path("hex-cli/src/commands/sched.rs"));
        assert!(is_critical_path("/var/home/user/hex-intf/hex-cli/src/commands/sched.rs"));
        assert!(is_critical_path("hex-agent/src/workplan_executor.rs"));
        assert!(!is_critical_path("hex-agent/src/test_utils.rs"));
        assert!(!is_critical_path("docs/guides/example.md"));
    }
}
