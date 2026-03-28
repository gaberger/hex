

use std::process::Command;
use std::io::Write;

#[test]
fn prints_hello_world() {
    let output = Command::new(env!("CARGO_BIN_EXE_build-cli-adapter-for-todo-list"))
        .output()
        .expect("failed to run binary");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "Hello from Build CLI Adapter for Todo List");
}

#[test]
fn error_case_not_applicable() {
    // No error cases in main function
}

#[test]
fn edge_case_not_applicable() {
    // No edge cases in main function
}