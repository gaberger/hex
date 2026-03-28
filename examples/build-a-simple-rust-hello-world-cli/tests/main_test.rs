

use std::process::Command;

#[test]
fn it_prints_hello_world() {
    let output = Command::new(env!("CARGO_BIN_EXE_hello-cli-implementation"))
        .output()
        .expect("failed to run binary");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "Hello from Hello CLI Implementation");
}