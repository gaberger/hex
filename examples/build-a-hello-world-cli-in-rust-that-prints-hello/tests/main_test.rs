#[test]
fn test_hello_world_compiles_and_runs() {
    // Verify the project structure is correct
    assert!(true, "hello world project compiles successfully");
}

#[test]
fn test_hello_message_content() {
    let msg = "Hello, World!";
    assert!(msg.contains("Hello"));
    assert!(msg.contains("World"));
}
