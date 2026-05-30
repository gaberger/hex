use crate::core::ports::ImageStorePort;

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

    pub async fn execute(&self, bytes: &[u8], content_type: &str) -> Result<String, String> {
        if !self.content_type_whitelist.contains(&content_type) {
            return Err(format!("Unsupported content type: {}", content_type));
        }

        if bytes.len() > self.max_byte_size {
            return Err("File size exceeds the maximum allowed limit".to_string());
        }

        self.image_store
            .store_image(bytes.to_vec())
            .await
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::domain::DomainError;
    use async_trait::async_trait;

    struct StubStore {
        sha: Option<String>,
    }

    #[async_trait]
    impl ImageStorePort for StubStore {
        async fn store_image(&self, _image_data: Vec<u8>) -> Result<String, DomainError> {
            match &self.sha {
                Some(s) => Ok(s.clone()),
                None => Err(DomainError::InvalidStartingPrice),
            }
        }

        async fn get_image(&self, _hash: &str) -> Result<Vec<u8>, DomainError> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn test_upload_image_success() {
        let use_case = UploadImageUseCase::new(
            Box::new(StubStore { sha: Some("image_id".to_string()) }),
            vec!["image/jpeg", "image/png"],
            1024 * 1024, // 1MB
        );

        assert_eq!(use_case.execute(&[0; 1024], "image/jpeg").await.unwrap(), "image_id");
    }

    #[tokio::test]
    async fn test_upload_image_unsupported_content_type() {
        let use_case = UploadImageUseCase::new(
            Box::new(StubStore { sha: Some("image_id".to_string()) }),
            vec!["image/png"],
            1024 * 1024, // 1MB
        );

        assert_eq!(
            use_case.execute(&[0; 1024], "image/jpeg").await.unwrap_err(),
            "Unsupported content type: image/jpeg"
        );
    }

    #[tokio::test]
    async fn test_upload_image_exceeds_max_size() {
        let use_case = UploadImageUseCase::new(
            Box::new(StubStore { sha: Some("image_id".to_string()) }),
            vec!["image/jpeg"],
            1024, // 1KB
        );

        assert_eq!(
            use_case.execute(&[0; 1025], "image/jpeg").await.unwrap_err(),
            "File size exceeds the maximum allowed limit"
        );
    }
}