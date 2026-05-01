use std::path::Path;

pub mod safe_file_writer;

pub trait IFileWriter {
    fn write(&self, path: &Path, contents: &[u8]) -> std::io::Result<()>;
}

pub use self::safe_file_writer::SafeFileWriter;