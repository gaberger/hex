//! `hex service` — manage hex as systemd **user** services.
//!
//! Wraps `systemctl --user` so the daemon stack (SpacetimeDB + hex-nexus) is
//! owned by systemd: auto-restart on failure, correct start ordering, and
//! start-at-boot (via linger). This routes reboot-persistence through the hex
//! surface instead of hand-rolled unit files.
//!
//! Units generated:
//!   - `hex-stdb.service`  → spacetimedb-standalone on :3033 (readiness-gated)
//!   - `hex-nexus.service` → hex-nexus on :5555 (Requires=+After=hex-stdb)
//!
//! Both run their binaries in the **foreground** (Type=simple) so systemd tracks
//! the real process, rather than wrapping the self-daemonizing `hex … start`.

use std::path::PathBuf;

use clap::Subcommand;
use colored::Colorize;

/// Default SpacetimeDB listen port. The plain `hex stdb start` default of 3000
/// collides with common dev servers (Next.js) and rootless-container forwards;
/// the rest of the stack (nexus, doctor, publish) looks for stdb on 3033.
const DEFAULT_STDB_PORT: u16 = 3033;
/// Default hex-nexus port (dashboard + REST API).
const DEFAULT_NEXUS_PORT: u16 = 5555;

const STDB_UNIT: &str = "hex-stdb.service";
const NEXUS_UNIT: &str = "hex-nexus.service";

#[derive(Subcommand)]
pub enum ServiceAction {
    /// Generate systemd user units, reload, and enable+start at boot
    Install {
        /// SpacetimeDB listen port
        #[arg(long, default_value_t = DEFAULT_STDB_PORT)]
        stdb_port: u16,
        /// hex-nexus port
        #[arg(long, default_value_t = DEFAULT_NEXUS_PORT)]
        nexus_port: u16,
        /// Install + enable but don't start now
        #[arg(long)]
        no_start: bool,
        /// Overwrite existing unit files without prompting
        #[arg(long)]
        force: bool,
    },
    /// Stop, disable, and remove the systemd user units
    Uninstall,
    /// Start the stack (stdb, then nexus)
    Start,
    /// Stop the stack (nexus, then stdb)
    Stop,
    /// Restart hex-nexus — picks up a freshly-built binary
    Restart {
        /// Also restart SpacetimeDB
        #[arg(long)]
        all: bool,
    },
    /// Show status of both services
    Status,
    /// Tail service logs via journalctl
    Logs {
        /// Follow log output
        #[arg(short, long)]
        follow: bool,
        /// Number of lines to show
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: usize,
        /// Show SpacetimeDB logs instead of nexus
        #[arg(long)]
        stdb: bool,
    },
    /// Enable services at boot (without starting)
    Enable,
    /// Disable services at boot
    Disable,
}

pub async fn run(action: ServiceAction) -> anyhow::Result<()> {
    ensure_systemctl()?;
    match action {
        ServiceAction::Install { stdb_port, nexus_port, no_start, force } => {
            install(stdb_port, nexus_port, no_start, force).await
        }
        ServiceAction::Uninstall => uninstall().await,
        ServiceAction::Start => {
            systemctl(&["start", STDB_UNIT, NEXUS_UNIT])?;
            println!("{} hex services started", glyph().green());
            status().await
        }
        ServiceAction::Stop => {
            // Reverse dependency order: nexus first, then stdb.
            systemctl(&["stop", NEXUS_UNIT, STDB_UNIT])?;
            println!("{} hex services stopped", glyph().green());
            Ok(())
        }
        ServiceAction::Restart { all } => {
            if all {
                systemctl(&["restart", STDB_UNIT, NEXUS_UNIT])?;
                println!("{} hex-stdb + hex-nexus restarted", glyph().green());
            } else {
                systemctl(&["restart", NEXUS_UNIT])?;
                println!("{} hex-nexus restarted", glyph().green());
            }
            status().await
        }
        ServiceAction::Status => status().await,
        ServiceAction::Logs { follow, lines, stdb } => logs(follow, lines, stdb),
        ServiceAction::Enable => {
            systemctl(&["enable", STDB_UNIT, NEXUS_UNIT])?;
            ensure_linger();
            println!("{} hex services enabled at boot", glyph().green());
            Ok(())
        }
        ServiceAction::Disable => {
            systemctl(&["disable", STDB_UNIT, NEXUS_UNIT])?;
            println!("{} hex services disabled at boot", glyph().green());
            Ok(())
        }
    }
}

async fn install(stdb_port: u16, nexus_port: u16, no_start: bool, force: bool) -> anyhow::Result<()> {
    let dir = unit_dir()?;
    std::fs::create_dir_all(&dir)?;

    // Resolve absolute binary paths — systemd units run from a bare environment,
    // so relative/`$PATH`-implicit lookups can't be relied on.
    let stdb_bin = which("spacetimedb-standalone").ok_or_else(|| {
        anyhow::anyhow!(
            "spacetimedb-standalone not found on PATH or in ~/.local/bin.\n  \u{2192} Install SpacetimeDB, or run `hex stdb start` once to bootstrap it."
        )
    })?;
    let nexus_bin = resolve_nexus_bin().ok_or_else(|| {
        anyhow::anyhow!(
            "hex-nexus binary not found.\n  \u{2192} Build it: cargo build -p hex-nexus --release && install -m755 target/*/release/hex-nexus ~/.local/bin/"
        )
    })?;

    let home = home_dir()?;
    let workdir = std::env::current_dir()?;

    let stdb_path = dir.join(STDB_UNIT);
    let nexus_path = dir.join(NEXUS_UNIT);
    if !force && (stdb_path.exists() || nexus_path.exists()) {
        println!(
            "{} unit files already exist in {} — re-run with {} to overwrite",
            "!".yellow(),
            dir.display(),
            "--force".cyan()
        );
        return Ok(());
    }

    std::fs::write(&stdb_path, render_stdb_unit(&stdb_bin, &home, stdb_port))?;
    std::fs::write(&nexus_path, render_nexus_unit(&nexus_bin, &home, &workdir, stdb_port, nexus_port))?;
    println!("{} wrote {}", glyph().green(), stdb_path.display());
    println!("{} wrote {}", glyph().green(), nexus_path.display());

    systemctl(&["daemon-reload"])?;

    if no_start {
        systemctl(&["enable", STDB_UNIT, NEXUS_UNIT])?;
        ensure_linger();
        println!("{} installed + enabled at boot (not started; run `hex service start`)", glyph().cyan());
        return Ok(());
    }

    systemctl(&["enable", "--now", STDB_UNIT])?;
    systemctl(&["enable", "--now", NEXUS_UNIT])?;
    ensure_linger();
    println!("{} hex services installed, enabled, and started", glyph().green());
    status().await
}

async fn uninstall() -> anyhow::Result<()> {
    // disable --now stops and removes the boot symlinks in one shot. Ignore
    // errors (units may already be gone) so uninstall is idempotent.
    let _ = systemctl(&["disable", "--now", NEXUS_UNIT, STDB_UNIT]);
    let dir = unit_dir()?;
    for unit in [NEXUS_UNIT, STDB_UNIT] {
        let path = dir.join(unit);
        if path.exists() {
            std::fs::remove_file(&path)?;
            println!("{} removed {}", glyph().green(), path.display());
        }
    }
    systemctl(&["daemon-reload"])?;
    println!("{} hex services uninstalled", glyph().green());
    Ok(())
}

async fn status() -> anyhow::Result<()> {
    for (unit, label) in [(STDB_UNIT, "SpacetimeDB"), (NEXUS_UNIT, "hex-nexus")] {
        let active = systemctl_capture(&["is-active", unit]).unwrap_or_else(|_| "unknown".into());
        let enabled = systemctl_capture(&["is-enabled", unit]).unwrap_or_else(|_| "unknown".into());
        let active = active.trim();
        let enabled = enabled.trim();
        let icon = if active == "active" { "\u{25cf}".green() } else { "\u{25cb}".red() };
        let active_col = if active == "active" { active.green() } else { active.red() };
        println!("  {} {:<12} {} ({} at boot)", icon, label, active_col, enabled);
    }
    Ok(())
}

fn logs(follow: bool, lines: usize, stdb: bool) -> anyhow::Result<()> {
    let unit = if stdb { STDB_UNIT } else { NEXUS_UNIT };
    let mut args = vec![
        "--user".to_string(),
        "-u".to_string(),
        unit.to_string(),
        "-n".to_string(),
        lines.to_string(),
    ];
    if follow {
        args.push("-f".to_string());
    }
    let status = std::process::Command::new("journalctl").args(&args).status()?;
    if !status.success() {
        anyhow::bail!("journalctl exited with {status}");
    }
    Ok(())
}

// ── unit rendering ───────────────────────────────────────────────────────────

fn render_stdb_unit(bin: &PathBuf, home: &PathBuf, port: u16) -> String {
    let data_dir = home.join(".local/share/spacetime/data");
    let jwt_dir = home.join(".config/spacetime/");
    let ping = hex_core::SPACETIMEDB_PING_PATH;
    format!(
        "[Unit]\n\
         Description=hex SpacetimeDB (local coordination core, :{port})\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={bin} start --data-dir {data} --jwt-key-dir {jwt} --listen-addr 0.0.0.0:{port}\n\
         ExecStartPost=/bin/sh -c 'for i in $(seq 1 60); do curl -sf http://127.0.0.1:{port}{ping} >/dev/null 2>&1 && exit 0; sleep 1; done; echo \"stdb not ready on :{port}\" >&2; exit 1'\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         TimeoutStartSec=90\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        port = port,
        bin = bin.display(),
        data = data_dir.display(),
        jwt = jwt_dir.display(),
        ping = ping,
    )
}

fn render_nexus_unit(bin: &PathBuf, home: &PathBuf, workdir: &PathBuf, stdb_port: u16, nexus_port: u16) -> String {
    let ping = hex_core::SPACETIMEDB_PING_PATH;
    let path_env = format!(
        "{}/.local/bin:/usr/local/bin:{}/.cargo/bin:/usr/bin:/bin",
        home.display(),
        home.display()
    );
    format!(
        "[Unit]\n\
         Description=hex-nexus daemon (FS bridge + dashboard :{nexus_port})\n\
         After={stdb_unit} network-online.target\n\
         Requires={stdb_unit}\n\
         \n\
         [Service]\n\
         Type=simple\n\
         WorkingDirectory={workdir}\n\
         Environment=PATH={path_env}\n\
         ExecStartPre=/bin/sh -c 'for i in $(seq 1 30); do curl -sf http://127.0.0.1:{stdb_port}{ping} >/dev/null 2>&1 && exit 0; sleep 1; done; echo \"stdb unreachable on :{stdb_port}\" >&2; exit 1'\n\
         ExecStart={bin} --port {nexus_port} --bind 0.0.0.0\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         TimeoutStartSec=120\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        nexus_port = nexus_port,
        stdb_port = stdb_port,
        stdb_unit = STDB_UNIT,
        workdir = workdir.display(),
        path_env = path_env,
        bin = bin.display(),
        ping = ping,
    )
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Ogham glyph used across hex CLI output.
fn glyph() -> colored::ColoredString {
    "\u{2b21}".normal()
}

fn ensure_systemctl() -> anyhow::Result<()> {
    if which("systemctl").is_none() {
        anyhow::bail!(
            "systemctl not found — `hex service` requires systemd (Linux).\n  \u{2192} Use `hex nexus start` / `hex stdb start` on non-systemd hosts."
        );
    }
    Ok(())
}

fn unit_dir() -> anyhow::Result<PathBuf> {
    Ok(home_dir()?.join(".config/systemd/user"))
}

fn home_dir() -> anyhow::Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not determine home directory"))
}

/// Run `systemctl --user <args>`, failing on non-zero exit.
fn systemctl(args: &[&str]) -> anyhow::Result<()> {
    let status = std::process::Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()?;
    if !status.success() {
        anyhow::bail!("systemctl --user {} failed ({status})", args.join(" "));
    }
    Ok(())
}

/// Run `systemctl --user <args>` and capture stdout (trimmed). Does not fail on
/// non-zero exit — `is-active`/`is-enabled` return non-zero for inactive units
/// but still print a useful state word.
fn systemctl_capture(args: &[&str]) -> anyhow::Result<String> {
    let out = std::process::Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Best-effort: enable linger so user services start at boot without a login
/// session. Non-fatal — on locked-down hosts this needs root; we just advise.
fn ensure_linger() {
    let user = std::env::var("USER").unwrap_or_default();
    if user.is_empty() {
        return;
    }
    let already = std::process::Command::new("loginctl")
        .args(["show-user", &user, "--property=Linger", "--value"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "yes")
        .unwrap_or(false);
    if already {
        return;
    }
    let ok = std::process::Command::new("loginctl")
        .args(["enable-linger", &user])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        println!("{} enabled linger for {} (services start at boot)", glyph().green(), user);
    } else {
        println!(
            "  {} could not enable linger automatically — run: {}",
            "!".yellow(),
            format!("sudo loginctl enable-linger {}", user).cyan()
        );
    }
}

/// Resolve the hex-nexus binary for the unit. Prefers the *installed* copy
/// (`HEX_NEXUS_BIN`, then `~/.local/bin`, then `$PATH`) over `./target` builds,
/// since a systemd unit must reference a stable absolute path, not a build tree
/// that may be `cargo clean`ed out from under it.
fn resolve_nexus_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HEX_NEXUS_BIN") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Some(path);
        }
    }
    which("hex-nexus")
}

/// Locate an executable: `~/.local/bin` and `~/.cargo/bin` first (the install
/// targets), then each `$PATH` entry. Returns an absolute path.
fn which(bin: &str) -> Option<PathBuf> {
    if let Ok(home) = home_dir() {
        for sub in [".local/bin", ".cargo/bin"] {
            let cand = home.join(sub).join(bin);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in path_var.split(':').filter(|d| !d.is_empty()) {
        let cand = PathBuf::from(dir).join(bin);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdb_unit_has_listen_addr_and_readiness_gate() {
        let unit = render_stdb_unit(
            &PathBuf::from("/opt/bin/spacetimedb-standalone"),
            &PathBuf::from("/home/gary"),
            3033,
        );
        assert!(unit.contains("ExecStart=/opt/bin/spacetimedb-standalone start"));
        assert!(unit.contains("--listen-addr 0.0.0.0:3033"));
        assert!(unit.contains("--data-dir /home/gary/.local/share/spacetime/data"));
        assert!(unit.contains("ExecStartPost=")); // readiness gate present
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn nexus_unit_orders_after_and_requires_stdb() {
        let unit = render_nexus_unit(
            &PathBuf::from("/home/gary/.local/bin/hex-nexus"),
            &PathBuf::from("/home/gary"),
            &PathBuf::from("/home/gary/hex-intf"),
            3033,
            5555,
        );
        assert!(unit.contains("After=hex-stdb.service"));
        assert!(unit.contains("Requires=hex-stdb.service"));
        assert!(unit.contains("WorkingDirectory=/home/gary/hex-intf"));
        assert!(unit.contains("ExecStart=/home/gary/.local/bin/hex-nexus --port 5555 --bind 0.0.0.0"));
        // foreground: must NOT daemonize
        assert!(!unit.contains("--daemon"));
    }
}
