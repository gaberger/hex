

use std::process::Command;
use std::io::Write;

#[test]
fn it_prints_hello_world_to_stdout() {
    let binary = env!("CARGO_BIN_EXE_hello-world-cli-implementation");
    let output = Command::new(binary)
        .output()
        .expect("failed to run binary");
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout).trim();
    assert_eq!(stdout, "Hello, World!");
    
    assert!(output.stderr.is_empty());
}