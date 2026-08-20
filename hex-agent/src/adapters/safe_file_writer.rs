//! SafeFileWriter — secondary adapter for autonomous code writes.
//!
//! Two layers of defense, evaluated in order:
//!
//!   1. Static infrastructure allowlist (`hex_core::validation::is_critical_path`).
//!      Catches well-known load-bearing paths (sched.rs, monitor.rs, etc.)
//!      regardless of CWD.
//!
//!   2. **Trunk-detect predicate (ADR-2026-05-08-1126 P3.1).** If the writer's
//!      effective CWD resolves to the git trunk AND the process is not in
//!      operator-mode, every write is denied — full stop. Rationale: a
//!      background agent should never write to trunk; it must work in a
//!      worktree and merge through the gate. This catches hijacker classes
//!      that the static allowlist misses (e.g. `Cargo.toml`, `lib.rs`,
//!      `spacetime-modules/*/src/lib.rs`).
//!
//! Operator override: `HEX_OPERATOR_MODE=1` bypasses layer 2. Footgun guard:
//! the override is rejected if the process tree includes any
//! `hex-agent daemon` ancestor (a hijacked daemon spawning a subprocess
//! cannot whitewash itself).
use std::path::{Path, PathBuf};

use hex_core::ports::file_writer::IFileWriter;
use hex_core::validation::is_critical_path;

pub struct SafeFileWriter {
    /// Cached trunk root (absolute, canonicalized). `None` = not in a git
    /// repo, in which case layer 2 is skipped (still falls through layer 1).
    trunk: Option<PathBuf>,
}

impl SafeFileWriter {
    pub fn new() -> Self {
        Self {
            trunk: discover_trunk(),
        }
    }
}

impl Default for SafeFileWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl IFileWriter for SafeFileWriter {
    fn write_file(&self, path: &Path, content: &str) -> Result<(), String> {
        let path_str = path.to_string_lossy();

        // Layer 1: static infrastructure allowlist.
        if is_critical_path(&path_str) {
            return Err(format!(
                "Cannot modify critical infrastructure file: {}",
                path_str
            ));
        }

        // Layer 2: trunk-detect predicate.
        if let Some(ref trunk) = self.trunk {
            if cwd_is_trunk(trunk) && !operator_mode_active() {
                return Err(format!(
                    "Refusing trunk write per ADR-2026-05-08-1126: {} \
                     (CWD is the git trunk and HEX_OPERATOR_MODE is not active). \
                     Either run from a worktree or set HEX_OPERATOR_MODE=1 (operator only).",
                    path_str
                ));
            }
        }

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent dir for {}: {}", path_str, e))?;
            }
        }

        std::fs::write(path, content)
            .map_err(|e| format!("Failed to write {}: {}", path_str, e))
    }
}

// ── trunk-detect helpers ─────────────────────────────────────────────────

fn discover_trunk() -> Option<PathBuf> {
    let out = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    for line in stdout.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            return std::fs::canonicalize(p).ok();
        }
    }
    None
}

/// Resolve the current process CWD to its git toplevel (the worktree it's
/// in, if any) and compare to the trunk path. Both are canonicalized so
/// `/home/gary/...` vs `/var/home/gary/...` (Bazzite symlink) compare equal.
fn cwd_is_trunk(trunk: &Path) -> bool {
    let out = match std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };
    let cwd_top = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if cwd_top.is_empty() {
        return false;
    }
    let cwd_canon = std::fs::canonicalize(&cwd_top).unwrap_or_else(|_| PathBuf::from(&cwd_top));
    let trunk_canon = std::fs::canonicalize(trunk).unwrap_or_else(|_| trunk.to_path_buf());
    cwd_canon == trunk_canon
}

/// `HEX_OPERATOR_MODE=1` allows trunk writes — but ONLY if the process tree
/// does NOT include a `hex-agent daemon` ancestor. Footgun guard: a daemon
/// spawning a subprocess can't bypass the gate by setting the env var on
/// the child; we walk up the process tree and refuse the override if we
/// find a daemon ancestor.
fn operator_mode_active() -> bool {
    let env_ok = std::env::var("HEX_OPERATOR_MODE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !env_ok {
        return false;
    }
    if has_hex_agent_daemon_ancestor() {
        eprintln!(
            "SafeFileWriter: HEX_OPERATOR_MODE=1 ignored — process tree includes hex-agent daemon \
             (footgun guard per ADR-2026-05-08-1126 P3.1)"
        );
        return false;
    }
    true
}

/// Walk /proc/self up via PPID. If any ancestor's cmdline starts with
/// `hex-agent` and contains `daemon`, return true. Linux-specific (matches
/// the rest of hex-agent's deployment surface).
fn has_hex_agent_daemon_ancestor() -> bool {
    let mut pid: i64 = std::process::id() as i64;
    for _ in 0..32 {
        // Bound the walk; init = PID 1, stop there.
        let cmdline_path = format!("/proc/{}/cmdline", pid);
        if let Ok(raw) = std::fs::read(&cmdline_path) {
            // /proc cmdline is NUL-separated.
            let mut argv = raw.split(|b| *b == 0);
            let arg0 = argv.next().unwrap_or(b"");
            let arg0_str = String::from_utf8_lossy(arg0);
            // Does it look like hex-agent? Match by basename.
            let basename = arg0_str.rsplit('/').next().unwrap_or(&arg0_str);
            if basename == "hex-agent" {
                let argv_rest: Vec<String> = argv
                    .filter(|a| !a.is_empty())
                    .map(|a| String::from_utf8_lossy(a).to_string())
                    .collect();
                if argv_rest.iter().any(|a| a == "daemon") {
                    return true;
                }
            }
        }
        // Advance to parent.
        let stat_path = format!("/proc/{}/stat", pid);
        match std::fs::read_to_string(&stat_path) {
            Ok(s) => {
                // /proc/<pid>/stat has the form: "PID (comm) STATE PPID ..."
                // comm can contain spaces/parens; the field we need is after
                // the LAST close-paren.
                let after_paren = match s.rfind(')') {
                    Some(i) => &s[i + 1..],
                    None => break,
                };
                let parts: Vec<&str> = after_paren.split_whitespace().collect();
                // After the close-paren the next field is state, then PPID.
                if parts.len() < 2 {
                    break;
                }
                let ppid: i64 = match parts[1].parse() {
                    Ok(p) => p,
                    Err(_) => break,
                };
                if ppid <= 1 {
                    break;
                }
                pid = ppid;
            }
            Err(_) => break,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Layer 1 still works: critical paths blocked regardless of CWD.
    #[test]
    fn layer1_rejects_sched_rs() {
        let writer = SafeFileWriter::new();
        let result =
            writer.write_file(Path::new("hex-cli/src/commands/sched.rs"), "// stub\n");
        let err = result.expect_err("expected critical-path block");
        assert!(
            err.contains("critical") || err.contains("Refusing trunk"),
            "expected critical or trunk-refuse, got: {}",
            err
        );
    }

    #[test]
    fn layer1_rejects_workplan_executor_rs() {
        let writer = SafeFileWriter::new();
        let result = writer.write_file(
            Path::new("/var/home/gary/hex-intf/hex-agent/src/workplan_executor.rs"),
            "fn main() {}",
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("critical") || err.contains("Refusing trunk"),
            "expected critical or trunk-refuse, got: {}",
            err
        );
    }

    #[test]
    fn layer1_rejects_bare_basename() {
        let writer = SafeFileWriter::new();
        let result = writer.write_file(Path::new("monitor.rs"), "");
        assert!(result.is_err(), "bare basename should be rejected");
    }

    /// Layer 2 (trunk-detect) BLOCKS writes when CWD canonicalizes to trunk
    /// and operator mode is not active. cargo test runs with CWD inside the
    /// workspace, so `git rev-parse --show-toplevel` resolves to the trunk
    /// root regardless of whether tests run from `hex-agent/` or workspace
    /// root — that's the load-bearing assertion of the layer.
    #[test]
    fn layer2_blocks_trunk_writes_without_operator_mode() {
        // Make sure operator mode is OFF.
        std::env::remove_var("HEX_OPERATOR_MODE");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("subdir/foo.rs");
        let writer = SafeFileWriter::new();
        let result = writer.write_file(&path, "// content\n");
        let err = result.expect_err("expected layer-2 trunk-refuse");
        assert!(
            err.contains("Refusing trunk write"),
            "expected trunk-refuse, got: {}",
            err
        );
    }


    /// Cargo test parallelism makes it unsafe to rely on env state across
    /// tests, so we collapse the operator-mode set+check into one assertion
    /// that snapshots and restores the env var around its critical section.
    /// In practice the operator-mode path is also covered manually + the
    /// integration tests in `daemon_worktree_required.rs`.
    #[test]
    fn layer2_operator_mode_active_when_env_set_and_no_daemon_ancestor() {
        let prev = std::env::var("HEX_OPERATOR_MODE").ok();
        std::env::set_var("HEX_OPERATOR_MODE", "1");
        let active = operator_mode_active();
        match prev {
            Some(v) => std::env::set_var("HEX_OPERATOR_MODE", v),
            None => std::env::remove_var("HEX_OPERATOR_MODE"),
        }
        assert!(
            active,
            "HEX_OPERATOR_MODE=1 should activate (cargo test process has no hex-agent daemon ancestor)"
        );
    }

    /// has_hex_agent_daemon_ancestor() walks /proc/self up. The test
    /// process is `cargo test ... daemon_worktree_required`, NOT a daemon,
    /// so the helper should return false.
    #[test]
    fn footgun_guard_correctly_reports_no_daemon_ancestor() {
        assert!(
            !has_hex_agent_daemon_ancestor(),
            "cargo test should not have a hex-agent daemon ancestor"
        );
    }

    /// Trunk discovery finds *some* path when run inside the workspace.
    #[test]
    fn discover_trunk_finds_workspace_root() {
        let trunk = discover_trunk();
        assert!(
            trunk.is_some(),
            "should discover trunk when run inside a workspace member"
        );
        let p = trunk.unwrap();
        assert!(
            p.exists() && p.is_dir(),
            "discovered trunk should be a directory: {:?}",
            p
        );
    }
}
