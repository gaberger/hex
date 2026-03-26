// tests/main_test.rs
use std::process::Stdio;
use std::io::Write;

// Since this is an integration test in the `tests/` directory, we need to import the crate's public items
// The binary name from Cargo.toml is "rust-cli-hello-world-implementation"
// We'll test the binary's behavior by running it as a process

#[test]
fn prints_hello_world_message() {
    // Test the binary's output when run normally
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rust-cli-hello-world-implementation"))
        .output()
        .expect("failed to run binary");
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "Hello from Rust CLI Hello World Implementation");
}

#[test]
fn handles_empty_stdin_gracefully() {
    // Test the binary's behavior when stdin is empty
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rust-cli-hello-world-implementation"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn binary");
    
    // Close stdin without writing anything
    child.stdin.take();
    
    let output = child.wait_with_output().expect("failed to wait for binary");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "Hello from Rust CLI Hello World Implementation");
}

#[test]
fn handles_stdin_input_without_crashing() {
    // Test the binary's behavior when stdin has some input
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rust-cli-hello-world-implementation"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn binary");
    
    // Write some input to stdin
    child.stdin.as_mut().unwrap().write_all(b"Hello from stdin\n").unwrap();
    
    let output = child.wait_with_output().expect("failed to wait for binary");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "Hello from Rust CLI Hello World Implementation");
}

#[test]
fn handles_large_stdin_input_without_crashing() {
    // Test the binary's behavior with large input
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rust-cli-hello-world-implementation"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn binary");
    
    // Write a large amount of data to stdin
    let large_input = vec![b'X'; 100000].into_iter().collect::<Vec<_>>();
    child.stdin.as_mut().unwrap().write_all(&large_input).unwrap();
    
    let output = child.wait_with_output().expect("failed to wait for binary");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "Hello from Rust CLI Hello World Implementation");
}