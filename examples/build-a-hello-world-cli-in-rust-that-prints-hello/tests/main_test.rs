use std::process::Stdio;
use std::io::Write;

#[test]
fn prints_hello_world_message() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rust-cli-hello-world-implementation"))
        .output()
        .expect("failed to run binary");
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "Hello from Rust CLI Hello World Implementation");
}

#[test]
fn handles_empty_stdin_gracefully() {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rust-cli-hello-world-implementation"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn binary");
    
    child.stdin.as_mut().unwrap().write_all(b"").unwrap();
    let output = child.wait_with_output().expect("failed to wait");
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "Hello from Rust CLI Hello World Implementation");
}

#[test]
fn ignores_stdin_content_and_still_prints_message() {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rust-cli-hello-world-implementation"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn binary");
    
    child.stdin.as_mut().unwrap().write_all(b"some random input\n").unwrap();
    let output = child.wait_with_output().expect("failed to wait");
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "Hello from Rust CLI Hello World Implementation");
}