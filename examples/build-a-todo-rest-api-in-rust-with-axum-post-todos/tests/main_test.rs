use std::process::Command;

#[test]
fn it_prints_starting_message() {
    let output = Command::new(env!("CARGO_BIN_EXE_implement-todo-rest-api-with-axum-and-he"))
        .output()
        .expect("failed to run binary");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "Starting Todo REST API...\nPress Ctrl+C to stop the server.");
}