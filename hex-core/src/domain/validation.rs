/// Critical system files that should not be modified
const CRITICAL_PATHS: &[&str] = &[
    "/etc/passwd",
    "/etc/shadow",
    "/boot/grub2/grub.cfg",
    "/etc/sudoers",
];

/// Critical hex infrastructure files autonomous agents cannot modify
/// without explicit operator approval. Each entry is a path *suffix* —
/// matched against the trailing components of the candidate path so that
/// e.g. "hex-nexus/src/sched.rs" matches whether passed absolute,
/// repo-relative, or as bare basename.
pub const CRITICAL_FILES: &[&str] = &[
    "sched.rs",
    "monitor.rs",
    "workplan_executor.rs",
    "main.rs",
];

/// Critical hex *path-prefix* guards. Each entry matches the
/// `parent_segments/` portion of a candidate path so that any file BELOW
/// these directories is treated as critical (e.g. `agents/hex/hex/cto.yml`
/// matches `agents/hex/hex/`).
///
/// Per ADR-2026-05-23-0900 §5 (CRITICAL_PREFIXES hardening): the persona
/// YAMLs were writable by autonomous agents through the `code_patch` tool
/// because the file `code_patch.rs` allowlist permits `hex-cli/assets/`.
/// Adversarial review of the rejected v0 persona-prompts ADR
/// (2026-05-23-0815) surfaced this as one leg of a privilege-escalation
/// chain that could let an attacker mutate the YAML "trust anchor" and
/// then have its body re-seeded as the supervisor's authoritative prompt.
/// Closing this prefix shuts the loop at the foundation, independent of
/// any future improver code paths.
pub const CRITICAL_PREFIXES: &[&str] = &[
    "agents/hex/hex/",
];

/// Check if a path is a critical system file OR a critical hex
/// infrastructure file OR sits under a critical path prefix. Used by
/// validation rules to prevent modification of sensitive files by
/// autonomous agents (SafeFileWriter adapter, code_patch tool).
pub fn is_critical_path(path: &str) -> bool {
    if CRITICAL_PATHS.contains(&path) {
        return true;
    }
    let normalized = path.trim_end_matches('/');
    let file_match = CRITICAL_FILES.iter().any(|&infra| {
        normalized == infra
            || normalized.ends_with(&format!("/{}", infra))
    });
    if file_match {
        return true;
    }
    CRITICAL_PREFIXES.iter().any(|&prefix| {
        // Prefix matches as a directory chunk anywhere in the path. We
        // intentionally do NOT require the prefix at the start — repo-
        // relative, absolute, and embedded-asset paths all pass through
        // the same guard.
        normalized.contains(prefix)
    })
}

/// ValidationRule trait defines the contract for validation logic.
/// All validation rules must be Send + Sync for use in async/threaded contexts.
pub trait ValidationRule: Send + Sync {
    fn validate(&self, path: &str, content: &str) -> Result<(), String>;
}

/// CriticalPathRule encapsulates the existing is_critical_path logic,
/// checking if a path matches system-critical files.
pub struct CriticalPathRule;

impl ValidationRule for CriticalPathRule {
    fn validate(&self, path: &str, _content: &str) -> Result<(), String> {
        if is_critical_path(path) {
            Err(format!("Cannot modify critical system file: {}", path))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critical_files_catches_sched_main_etc() {
        assert!(is_critical_path("sched.rs"));
        assert!(is_critical_path("hex-nexus/src/sched.rs"));
        assert!(is_critical_path("/abs/path/main.rs"));
        assert!(is_critical_path("workplan_executor.rs"));
    }

    #[test]
    fn critical_prefixes_catches_persona_yamls() {
        // The 2026-05-23 v1 persona-prompts ADR finding: persona YAMLs
        // must be guarded. These all sit under agents/hex/hex/ and must
        // be rejected by is_critical_path.
        assert!(is_critical_path("hex-cli/assets/agents/hex/hex/cto.yml"));
        assert!(is_critical_path("agents/hex/hex/cpo.yml"));
        assert!(is_critical_path("/abs/foo/agents/hex/hex/ciso.yml"));
    }

    #[test]
    fn non_critical_paths_pass() {
        assert!(!is_critical_path("hex-nexus/src/orchestration/org_responder.rs"));
        assert!(!is_critical_path("docs/specs/foo.md"));
        assert!(!is_critical_path("hex-cli/assets/wasm/hexflo_coordination.wasm"));
        assert!(!is_critical_path("README.md"));
    }

    #[test]
    fn system_paths_are_critical() {
        assert!(is_critical_path("/etc/passwd"));
        assert!(is_critical_path("/etc/shadow"));
    }
}