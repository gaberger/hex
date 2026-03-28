

use std::process::Command;
use std::io::Write;

#[test]
fn prints_hello_world() {
    let output = Command::new(env!("CARGO_BIN_EXE_hello-world-cli-implementation"))
        .output()
        .expect("Failed to run the binary");
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout).trim();
    assert_eq!(stdout, "Hello, World!");
}