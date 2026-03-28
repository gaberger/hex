

use std::process::Command;
use std::io::Write;

#[test]
fn it_prints_hello_world() {
    let output = Command::new(env!("CARGO_BIN_EXE_rust-cli-hello-world-implementation"))
        .output()
        .expect("failed to run binary");
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "Hello from Rust CLI Hello World Implementation");
}

#[test]
fn it_handles_no_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_rust-cli-hello-world-implementation"))
        .output()
        .expect("failed to run binary");
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "Hello from Rust CLI Hello World Implementation");
}

#[test]
fn it_handles_multiple_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_rust-cli-hello-world-implementation"))
        .args(&["arg1", "arg2"])
        .output()
        .expect("failed to run binary");
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "Hello from Rust CLI Hello World Implementation");
}