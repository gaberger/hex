use core::ports::ImageStorePort;
use std::path::Path;

#[derive(Debug)]
pub struct UploadImageUseCase {
    image_store: Box<dyn ImageStorePort>,
    content_type_whitelist: Vec<&'static str>,
    max_byte_size: usize,
}

impl UploadImageUseCase {
    pub fn new(
        image_store: Box<dyn ImageStorePort>,
        content_type_whitelist: Vec<&'static str>,
        max_byte_size: usize,
    ) -> Self {
        Self {
            image_store,
            content_type_whitelist,
            max_byte_size,
        }
    }

    pub fn execute(&self, bytes: &[u8], content_type: &str) -> Result<String, String> {
        if !self.content_type_whitelist.contains(&content_type) {
            return Err(format!("Unsupported content type: {}", content_type));
        }

        if bytes.len() > self.max_byte_size {
            return Err("File size exceeds the maximum allowed limit".to_string());
        }

        let image_id = self.image_store.store(bytes, content_type)?;
        Ok(image_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::mock;
    use std::io;

    mock! {
        ImageStorePort {}

        impl ImageStorePort for ImageStorePort {
            fn store(&self, bytes: &[u8], content_type: &str) -> Result<String, String>;
        }
    }

    #[test]
    fn test_upload_image_success() {
        let mut mock_store = MockImageStorePort::new();
        mock_store.expect_store().returning(|_, _| Ok("image_id".to_string()));

        let use_case = UploadImageUseCase::new(
            Box::new(mock_store),
            vec!["image/jpeg", "image/png"],
            1024 * 1024, // 1MB
        );

        let result = use_case.execute(&[1, 2, 3], "image/jpeg");
        assert_eq!(result.unwrap(), "image_id".to_string());
    }

    #[test]
    fn test_upload_image_invalid_content_type() {
        let mock_store = MockImageStorePort::new();
        let use_case = UploadImageUseCase::new(
            Box::new(mock_store),
            vec!["image/jpeg", "image/png"],
            1024 * 1024, // 1MB
        );

        let result = use_case.execute(&[1, 2, 3], "image/gif");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Unsupported content type: image/gif".to_string());
    }

    #[test]
    fn test_upload_image_exceeds_max_size() {
        let mock_store = MockImageStorePort::new();
        let use_case = UploadImageUseCase::new(
            Box::new(mock_store),
            vec!["image/jpeg", "image/png"],
            5, // 5 bytes
        );

        let result = use_case.execute(&[1, 2, 3, 4, 5, 6], "image/jpeg");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "File size exceeds the maximum allowed limit".to_string());
    }

    #[test]
    fn test_upload_image_idempotency() {
        let mut mock_store = MockImageStorePort::new();
        mock_store.expect_store().returning(|_, _| Ok("image_id".to_string()));

        let use_case = UploadImageUseCase::new(
            Box::new(mock_store),
            vec!["image/jpeg", "image/png"],
            1024 * 1024, // 1MB
        );

        let result1 = use_case.execute(&[1, 2, 3], "image/jpeg");
        let result2 = use_case.execute(&[1, 2, 3], "image/jpeg");

        assert_eq!(result1.unwrap(), "image_id".to_string());
        assert_eq!(result2.unwrap(), "image_id".to_string());
    }
}

// docs/specs/ebay-spec-010
// docs/specs/ebay-spec-011