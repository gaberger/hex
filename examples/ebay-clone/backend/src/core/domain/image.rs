use serde::{Deserialize, Serialize};
use std::fmt;

// ADR-2026-05-19-0721: Specifies the structure of ImageRef for domain consistency.

/// Represents a reference to an image in the system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    /// SHA-256 hash of the image content.
    pub sha256: String,
    /// MIME type of the image.
    pub content_type: String,
    /// Size of the image in bytes.
    pub byte_size: u64,
}

impl fmt::Display for ImageRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ImageRef(sha256: {}, content_type: {}, size: {})", self.sha256, self.content_type, self.byte_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_ref_display() {
        let image = ImageRef {
            sha256: "a1b2c3d4e5f6".to_string(),
            content_type: "image/jpeg".to_string(),
            byte_size: 1024,
        };
        assert_eq!(format!("{}", image), "ImageRef(sha256: a1b2c3d4e5f6, content_type: image/jpeg, size: 1024)");
    }
}