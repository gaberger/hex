//! Tests for hex-desktop Tauri commands (unit-testable without Tauri runtime).
//!
//! The commands module exposes pure functions that don't require a Tauri
//! AppHandle except for `open_project`. We test the data-returning commands
//! directly and verify serialization contracts.

/// Verify HubStatus serialization matches the JS contract.
#[test]
fn hub_status_serialization() {
    // Simulate what get_hub_status returns
    let status = serde_json::json!({
        "running": true,
        "port": hex_nexus::DEFAULT_PORT,
        "version": hex_nexus::version(),
        "buildHash": hex_nexus::build_hash(),
    });

    assert_eq!(status["running"], true);
    assert_eq!(status["port"], 5555);
    assert!(status["version"].is_string());
    assert!(status["buildHash"].is_string());
}

/// Verify version string format.
#[test]
fn version_string_format() {
    let version = format!(
        "hex-desktop {} ({})",
        hex_nexus::version(),
        hex_nexus::build_hash()
    );

    assert!(version.starts_with("hex-desktop "));
    assert!(version.contains('('));
    assert!(version.contains(')'));
}

/// Verify that hex-nexus re-exports are accessible from hex-desktop context.
#[test]
fn core_reexports_accessible() {
    assert_eq!(hex_nexus::DEFAULT_PORT, 5555);
    assert!(!hex_nexus::version().is_empty());
    assert!(!hex_nexus::build_hash().is_empty());
}

/// Verify HubConfig defaults.
#[test]
fn hub_config_defaults() {
    let config = hex_nexus::HubConfig::default();
    assert_eq!(config.port, 5555);
    assert_eq!(config.bind, "127.0.0.1");
    assert!(config.token.is_none());
    assert!(!config.is_daemon);
}

/// Verify daemon lock file utilities are accessible.
#[test]
fn daemon_lock_path_is_deterministic() {
    let path1 = hex_nexus::daemon::lock_file_path();
    let path2 = hex_nexus::daemon::lock_file_path();
    assert_eq!(path1, path2);
    assert!(path1.to_string_lossy().contains(".hex"));
    assert!(path1.to_string_lossy().contains("hub.lock"));
}

/// Verify token generation produces valid hex strings.
#[test]
fn generated_token_is_valid_hex() {
    let token = hex_nexus::daemon::generate_token();
    assert_eq!(token.len(), 32); // 16 bytes → 32 hex chars
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
}

/// Verify two generated tokens are unique.
#[test]
fn generated_tokens_are_unique() {
    let t1 = hex_nexus::daemon::generate_token();
    let t2 = hex_nexus::daemon::generate_token();
    assert_ne!(t1, t2);
}
