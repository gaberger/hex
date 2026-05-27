// hex-cli/tests/standalone_gate.rs

use std::env;
use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Command, Stdio};
use std::time::Duration;
use std::path::Path;

#[test]
fn test_standalone_gate() {
    let hex_nexus_url = env::var("HEX_NEXUS_URL").unwrap_or_else(|_| "http://127.0.0.1:5555".to_string());
    let ollama_host = env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());

    fn is_reachable(host_port: &str) -> bool {
        let timeout = Duration::new(2, 0);
        match host_port.to_socket_addrs() {
            Ok(addrs) => addrs.any(|addr| TcpStream::connect_timeout(&addr, timeout).is_ok()),
            Err(_) => false,
        }
    }

    if !is_reachable(&hex_nexus_url.split("//").nth(1).unwrap_or_default()) || !is_reachable(&ollama_host.split("//").nth(1).unwrap_or_default()) {
        eprintln!("One or both endpoints are unreachable, skipping test.");
        return;
    }

    let output = Command::new("bash")
        .arg("run.sh")
        .arg("--tier")
        .arg("T1")
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/standalone-pipeline-test"))
        .env("HEX_NEXUS_URL", &hex_nexus_url)
        .env("OLLAMA_HOST", &ollama_host)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run run.sh");

    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(capture) = stdout.lines().find(|l| l.contains("Results: ")) {
        let parts: Vec<&str> = capture.split_whitespace().collect();
        if let Ok(n_passed) = parts[1].parse::<u32>() {
            assert!(n_passed >= 2, "Expected at least 2 tests to pass, but only {} passed", n_passed);
        }
    }

    assert!(output.status.success(), "run.sh did not exit successfully");
}
