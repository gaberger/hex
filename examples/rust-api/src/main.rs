mod adapters;
mod core;

use adapters::primary::http_adapter::HttpAdapter;
use adapters::secondary::memory_store::MemoryStoreAdapter;

fn main() {
    // Composition root: wire adapters to ports
    let store = MemoryStoreAdapter::new();
    let http = HttpAdapter::new(store);

    // Demo usage
    println!("Rust Hex API - Key-Value Store");
    let _ = http.handle_set("greeting", "hello world");
    match http.handle_get("greeting") {
        Ok(result) => println!("{}", result),
        Err(e) => eprintln!("Error: {}", e.message),
    }
    match http.handle_list() {
        Ok(result) => println!("All entries:\n{}", result),
        Err(e) => eprintln!("Error: {}", e.message),
    }
}
