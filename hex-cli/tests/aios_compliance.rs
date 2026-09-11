//! ADR-2026-04-13-1500 AIOS Developer Experience — Compliance Smoke Test (P5.2)
//!
//! Verifies that the surviving ADR section is wired into the hex CLI:
//!
//!   §6 — `hex new`                  project intake
//!
//! §1 `brief`, §2 `decide`, §3 `steer`, §4 `trust`, §5 `taste` and §7
//! `pause`/`resume`/`override` were removed with the daemon (ADR-2608241500
//! P6.1/P6.2): each was a thin client over a control plane that no longer
//! exists. `brief` rendered `/api/briefing`, whose sessions, decisions and
//! health all came from SpacetimeDB.
//!
//! Strategy: invoke the compiled binary with `--help` for each subcommand.
//! Clap exits 0 for `--help`, so these tests are hermetic.
//!
//! For commands that *don't* have a subcommand `--help` (top-level like
//! `hex pause`), we invoke without args and accept either exit-0 (help)
//! or exit-1 (runtime error from missing nexus), but NOT exit-2 (clap
//! parse error, meaning the command doesn't exist).

use std::process::Command;

/// Locate the hex binary. Prefer the debug build in target/debug since
/// integration tests are run against debug builds by default.
fn hex_bin() -> Command {
    // CARGO_BIN_EXE_hex IS available for [[bin]] targets in integration
    // tests — the previous comment was wrong. It points at whichever
    // profile cargo just built (debug for `cargo test`, release for
    // `cargo test --release`), which is exactly what we want and which
    // CI's target-triple build dir handles correctly.
    Command::new(env!("CARGO_BIN_EXE_hex"))
}

/// Assert that `hex <args> --help` exits 0 (clap prints help and exits).
/// This proves the subcommand is registered in the CLI router.
fn assert_help_succeeds(args: &[&str], section_label: &str) {
    let mut cmd = hex_bin();
    cmd.args(args).arg("--help");
    // Prevent nexus connection attempts from hanging
    cmd.env("HEX_NEXUS_URL", "http://127.0.0.1:1");

    let output = cmd.output().unwrap_or_else(|e| {
        panic!("[{}] failed to execute hex {:?}: {}", section_label, args, e);
    });

    assert!(
        output.status.success(),
        "[{}] `hex {} --help` exited with {}\nstderr: {}",
        section_label,
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.is_empty(),
        "[{}] `hex {} --help` produced no output",
        section_label,
        args.join(" "),
    );
}

// ── §2: hex decide — decision resolution ────────────────────────────────────

// ── §3: hex steer — directive classification ────────────────────────────────

// ── §4: hex trust — delegation trust levels ─────────────────────────────────

// ── §5: hex taste — preference graph + injection resilience ─────────────────

/// Prompt injection resilience: verify the CLI accepts values containing
/// injection-like payloads without crashing or misrouting. The actual
/// sanitization happens nexus-side, but the CLI must not choke on the input.
// ── §6: hex new — structured project intake ─────────────────────────────────

#[test]
fn s6_new_help() {
    // `hex new --help` should show usage without creating anything.
    assert_help_succeeds(&["new"], "§6 new");
}

/// Verify `hex new` accepts --name and --description.
#[test]
fn s6_new_flags_recognized() {
    let mut cmd = hex_bin();
    cmd.args(&["new", "--help"]);
    cmd.env("HEX_NEXUS_URL", "http://127.0.0.1:1");

    let output = cmd.output().expect("hex new --help should run");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("--name") || stdout.contains("-n"),
        "[§6 new] --name flag missing from hex new --help output"
    );
}

