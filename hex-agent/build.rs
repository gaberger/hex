use std::process::Command;

fn main() {
    // Generate a build hash from the git commit, matching hex-hub's ADR-016 pattern.
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=HEX_AGENT_BUILD_HASH={}", hash);
    println!("cargo:rerun-if-changed=.git/HEAD");
}
