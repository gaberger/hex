use crate::core::domain::DomainError;
use crate::core::ports::ImageStorePort;

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

    /// Validates the upload against the content-type whitelist and size cap,
    /// then delegates to the `ImageStorePort`. Conforms to the port contract:
    /// `store_image(Vec<u8>) -> Result<String, DomainError>` (async).
    pub async fn execute(&self, bytes: &[u8], content_type: &str) -> Result<String, DomainError> {
        if !self.content_type_whitelist.contains(&content_type) {
            return Err(DomainError::Internal(format!(
                "Unsupported content type: {}",
                content_type
            )));
        }

        if bytes.len() > self.max_byte_size {
            return Err(DomainError::Internal(
                "File size exceeds the maximum allowed limit".to_string(),
            ));
        }

        let image_id = self.image_store.store_image(bytes.to_vec()).await?;
        Ok(image_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct StubImageStore;

    #[async_trait]
    impl ImageStorePort for StubImageStore {
        async fn store_image(&self, _image_data: Vec<u8>) -> Result<String, DomainError> {
            Ok("image_id".to_string())
        }

        async fn get_image(&self, _hash: &str) -> Result<Vec<u8>, DomainError> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn test_upload_image_success() {
        let use_case = UploadImageUseCase::new(
            Box::new(StubImageStore),
            vec!["image/jpeg", "image/png"],
            1024 * 1024, // 1MB
        );

        assert_eq!(
            use_case.execute(&[0; 1024], "image/jpeg").await.unwrap(),
            "image_id"
        );
    }

    #[tokio::test]
    async fn test_upload_image_unsupported_content_type() {
        let use_case = UploadImageUseCase::new(
            Box::new(StubImageStore),
            vec!["image/png"],
            1024 * 1024, // 1MB
        );

        assert!(use_case.execute(&[0; 1024], "image/jpeg").await.is_err());
    }

    #[tokio::test]
    async fn test_upload_image_exceeds_max_size() {
        let use_case = UploadImageUseCase::new(
            Box::new(StubImageStore),
            vec!["image/jpeg"],
            1024, // 1KB
        );

        assert!(use_case.execute(&[0; 1025], "image/jpeg").await.is_err());
    }
}