// hex-cli/tests/standalone_gate_v2.rs

use std::env;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;
use std::process::Command;

fn reachable(url: &str) -> bool {
    let host_port = if let Some(pos) = url.find("//") { &url[pos + 2..] } else { return false; };
    let host_port = if let Some(pos) = host_port.find('/') { &host_port[..pos] } else { host_port };
    match host_port.to_socket_addrs() {
        Ok(mut addrs) => addrs.any(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok()),
        Err(_) => false,
    }
}

#[test]
fn standalone_gate_smoke() {
    let hex_nexus_url = env::var("HEX_NEXUS_URL").unwrap_or_else(|_| "http://127.0.0.1:5555".to_string());
    let ollama_host = env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());

    if !reachable(&hex_nexus_url) || !reachable(&ollama_host) {
        eprintln!("Skipping test because either HEX_NEXUS_URL or OLLAMA_HOST is not reachable");
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
        .expect("run.sh failed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(line) = stdout.lines().find(|line| line.contains("Results:")) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if let Ok(n) = parts[1].split('/').next().unwrap_or_default().parse::<u32>() {
            assert!(n >= 2);
            assert!(output.status.success());
        } else {
            panic!("Failed to parse number of passed tests from output");
        }
    } else {
        panic!("Did not find 'Results:' line in output");
    }
}