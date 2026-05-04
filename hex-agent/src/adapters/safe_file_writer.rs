//! SafeFileWriter — secondary adapter for autonomous code writes.
//!
//! Refuses to clobber hex infrastructure files (sched.rs, monitor.rs,
//! workplan_executor.rs, main.rs, …) by consulting
//! `hex_core::validation::is_critical_path`. Used by
//! `workplan_executor::execute_workplan_autonomous` so that a runaway
//! background agent cannot satisfy literal-grep evidence by overwriting
//! the very files that drive the autonomy loop. See workplan
//! `wp-safe-file-writer-adapter` and CLAUDE.md "Background hex-agent
//! overwrites files" lesson.
use std::path::Path;

use hex_core::ports::file_writer::IFileWriter;
use hex_core::validation::is_critical_path;

pub struct SafeFileWriter;

impl SafeFileWriter {
    pub fn new() -> Self {
        Self
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
        if is_critical_path(&path_str) {
            return Err(format!(
                "Cannot modify critical infrastructure file: {}",
                path_str
            ));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_sched_rs_in_repo_relative_path() {
        let writer = SafeFileWriter::new();
        let result =
            writer.write_file(Path::new("hex-cli/src/commands/sched.rs"), "// stub\n");
        let err = result.expect_err("expected critical-path block");
        assert!(
            err.contains("critical"),
            "error must contain 'critical', got: {}",
            err
        );
    }

    #[test]
    fn rejects_workplan_executor_rs() {
        let writer = SafeFileWriter::new();
        let result = writer.write_file(
            Path::new("/var/home/gary/hex-intf/hex-agent/src/workplan_executor.rs"),
            "fn main() {}",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("critical"));
    }

    #[test]
    fn rejects_bare_basename() {
        let writer = SafeFileWriter::new();
        let result = writer.write_file(Path::new("monitor.rs"), "");
        assert!(result.is_err(), "bare basename should be rejected");
    }

    #[test]
    fn allows_normal_files_under_tmp() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("subdir/foo.rs");
        let writer = SafeFileWriter::new();
        writer
            .write_file(&path, "// safe content\n")
            .expect("normal write should succeed");
        let read = std::fs::read_to_string(&path).expect("file should exist");
        assert_eq!(read, "// safe content\n");
    }
}
