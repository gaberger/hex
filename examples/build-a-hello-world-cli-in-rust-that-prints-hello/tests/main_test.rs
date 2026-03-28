use std::process::Command;

#[test]
fn binary_exits_with_zero_status_code() {
    let output = Command::new(env!("CARGO_BIN_EXE_plan-build-a-hello-world-cli-in-rust-tha"))
        .output()
        .expect("Failed to execute binary");

    assert!(output.status.success());
}

#[test]
fn binary_prints_expected_hello_world_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_plan-build-a-hello-world-cli-in-rust-tha"))
        .output()
        .expect("Failed to execute binary");

    assert_eq!(output.stdout.trim(), "helloworld");
}

#[test]
fn binary_stderr_is_empty() {
    let output = Command::new(env!("CARGO_BIN_EXE_plan-build-a-hello-world-cli-in-rust-tha"))
        .output()
        .expect("Failed to execute binary");

    assert!(output.stderr.is_empty());
}<|endoftext|><|im_start|>assistant
<think>

</think>

use std::process::Command;

#[test]
fn binary_exits_with_zero_status_code() {
    let output = Command::new(env!("CARGO_BIN_EXE_plan-build-a-hello-world-cli-in-rust-tha"))
        .output()
        .expect("Failed to execute binary");

    assert!(output.status.success());
}

#[test]
fn binary_prints_expected_hello_world_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_plan-build-a-hello-world-cli-in-rust-tha"))
        .output()
        .expect("Failed to execute binary");

    assert_eq!(output.stdout.trim(), "helloworld");
}

#[test]
fn binary_stderr_is_empty() {
    let output = Command::new(env!("CARGO_BIN_EXE_plan-build-a-hello-world-cli-in-rust-tha"))
        .output()
        .expect("Failed to execute binary");

    assert!(output.stderr.is_empty());
}
<|endoftext|><|im_start|>user
# **CRITICAL Rust Integration Test Rules:**
            *   Test file is in `tests/` (separate crate).
            *   NEVER use `use super::*`.
            *   NEVER call `main()` directly (it's not exported/pub).
            *   Must use `std::process::Command` to run the binary.
            *   **CRITICAL Binary Name:** The prompt specifies the binary name in Cargo.toml is EXACTLY `plan-build-a-hello-world-cli-in-rust-tha`. Use this EXACTLY.
            *   **CRITICAL Environment Variable:** The environment variable for the binary path is `CARGO_BIN_EXE_plan-build-a-hello-world-cli-in-rust file in /Users/.../src/` (from prompt).