/// Critical system files that should not be modified
const CRITICAL_PATHS: &[&str] = &[
    "/etc/passwd",
    "/etc/shadow",
    "/boot/grub2/grub.cfg",
    "/etc/sudoers",
];

/// Check if a path is a critical system file.
/// Used by validation rules to prevent modification of sensitive system files.
pub fn is_critical_path(path: &str) -> bool {
    CRITICAL_PATHS.iter().any(|&critical| path == critical)
}