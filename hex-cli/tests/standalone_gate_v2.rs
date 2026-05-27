// hex-cli/tests/standalone_gate_v2.rs

use std::env;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;
use std::process::Command;

fn reachable(url: &str) -> bool {
    let url = if let Some(pos) = url.find("//") { &url[pos + 2..] } else { return false; };
    for addr in url.to_socket_addrs().unwrap() {
        if TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok() {
            return true;
        }
    }
    false
}

#[test]
fn standalone_gate_smoke() {
    let hex_nexus_url = env::var("HEX_NEXUS_URL").unwrap_or_else(|_| "http://127.0.0.1:5555".to_string());
    let ollama_host = env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());

    if !reachable(&hex_nexus_url) || !reachable(&ollama_host) {
        eprintln!("Skipping test: one or both endpoints are unreachable.");
        return;
    }

    let output = Command::new("bash")
        .arg("run.sh")
        .arg("--tier")
        .arg("T1")
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/standalone-pipeline-test"))
        .env("HEX_NEXUS_URL", &hex_nexus_url)
        .env("OLLAMA_HOST", &ollama_host)
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let results_line = stdout.lines().find(|line| line.contains("Results:"));

    if let Some(line) = results_line {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if let Ok(n) = parts[1].parse::<u32>() {
            assert!(n >= 2, "Expected at least 2 results but got {}", n);
        }
    }

    assert!(output.status.success(), "Command did not execute successfully");
}