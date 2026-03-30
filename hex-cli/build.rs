fn main() {
    // Tell Cargo to recompile when any embedded asset changes.
    // Without this, rust-embed embeds stale content after asset edits.
    println!("cargo:rerun-if-changed=assets/");
}
