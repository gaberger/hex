use std::path::PathBuf;

use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::core::domain::DomainError;
use crate::core::ports::image_store::ImageStorePort;

const DEFAULT_ROOT: &str = "examples/ebay-clone/backend/data/images";

pub struct FilesystemImageStore {
    root_dir: PathBuf,
}

impl Default for FilesystemImageStore {
    fn default() -> Self {
        FilesystemImageStore {
            root_dir: PathBuf::from(DEFAULT_ROOT),
        }
    }
}

impl FilesystemImageStore {
    pub fn new(root_dir: Option<String>) -> Self {
        FilesystemImageStore {
            root_dir: root_dir
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_ROOT)),
        }
    }

    fn file_path(&self, sha256: &str) -> PathBuf {
        self.root_dir.join(format!("{}.bin", sha256))
    }
}

#[async_trait::async_trait]
impl ImageStorePort for FilesystemImageStore {
    async fn store_image(&self, image_data: Vec<u8>) -> Result<String, DomainError> {
        let mut hasher = Sha256::new();
        hasher.update(&image_data);
        let sha256 = hex::encode(hasher.finalize());

        fs::create_dir_all(&self.root_dir)
            .await
            .map_err(|e| DomainError::Internal(e.to_string()))?;

        let file_path = self.file_path(&sha256);
        let mut file = fs::File::create(&file_path)
            .await
            .map_err(|e| DomainError::Internal(e.to_string()))?;
        file.write_all(&image_data)
            .await
            .map_err(|e| DomainError::Internal(e.to_string()))?;

        Ok(sha256)
    }

    async fn get_image(&self, hash: &str) -> Result<Vec<u8>, DomainError> {
        let file_path = self.file_path(hash);
        fs::read(&file_path)
            .await
            .map_err(|e| DomainError::Internal(e.to_string()))
    }
}