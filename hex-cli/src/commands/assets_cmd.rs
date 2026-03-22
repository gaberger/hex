//! `hex assets` — inspect embedded assets baked into the binary.

use colored::Colorize;

use crate::assets::Assets;

pub async fn list() -> anyhow::Result<()> {
    println!("{} Embedded Assets (ADR-2603221522)", "\u{2b21}".cyan());
    println!();

    let mut total_size: usize = 0;
    let mut count: usize = 0;

    let mut paths: Vec<_> = Assets::iter().collect();
    paths.sort();

    for path in &paths {
        if let Some(file) = Assets::get(path) {
            let size = file.data.len();
            total_size += size;
            count += 1;
            println!("  {} ({} bytes)", path, size);
        }
    }

    println!();
    println!(
        "  {} asset(s), {} bytes total",
        count,
        total_size
    );
    Ok(())
}
