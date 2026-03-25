use std::process::Command;

fn main() {
    // Ensure rust-embed folders exist so the Embed derive macro doesn't fail
    // in worktrees or CI where the frontend hasn't been built yet.
    let embed_dirs = [
        "assets/dist",
        "../hex-cli/assets/hooks",
        "../hex-cli/assets/mcp",
    ];
    for dir in &embed_dirs {
        let _ = std::fs::create_dir_all(dir);
    }
    println!("cargo:rerun-if-changed=assets/dist");
    println!("cargo:rerun-if-changed=../hex-cli/assets/hooks");
    println!("cargo:rerun-if-changed=../hex-cli/assets/mcp");

    // Embed git short hash at compile time
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output();

    let git_hash = match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }
        _ => "unknown".to_string(),
    };

    println!("cargo:rustc-env=HEX_HUB_BUILD_HASH={git_hash}");
    // Rebuild when git HEAD changes
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");
}
