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

/// Validate that workplan task files don't target critical infrastructure
/// Returns Err with list of blocked files if any critical paths detected
pub fn validate_workplan_task_safety(files: &[String]) -> Result<(), Vec<String>> {
    let mut blocked = Vec::new();

    for file in files {
        if is_critical_path(file) {
            blocked.push(file.clone());
        }
    }

    if blocked.is_empty() {
        Ok(())
    } else {
        Err(blocked)
    }
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

    #[test]
    fn test_workplan_task_safety_validation() {
        // Should pass for safe files
        let safe_files = vec![
            "docs/test.md".to_string(),
            "hex-agent/src/test_utils.rs".to_string(),
        ];
        assert!(validate_workplan_task_safety(&safe_files).is_ok());

        // Should fail for critical files
        let critical_files = vec![
            "hex-cli/src/commands/sched.rs".to_string(),
        ];
        assert!(validate_workplan_task_safety(&critical_files).is_err());

        // Should fail if mixed
        let mixed_files = vec![
            "docs/test.md".to_string(),
            "hex-agent/src/workplan_executor.rs".to_string(),
        ];
        let result = validate_workplan_task_safety(&mixed_files);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().len(), 1);
    }
}
