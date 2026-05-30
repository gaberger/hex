use std::fs;
use std::path::PathBuf;

use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::core::domain::DomainError;
use crate::core::ports::ImageStorePort;

mod hasher;

const MAX_FILE_SIZE: usize = 5 * 1024 * 1024; // 5 MiB

pub struct FilesystemImageStore {
    root_dir: PathBuf,
}

impl Default for FilesystemImageStore {
    fn default() -> Self {
        FilesystemImageStore {
            root_dir: PathBuf::from("examples/ebay-clone/backend/data/images"),
        }
    }
}

impl FilesystemImageStore {
    pub fn new(root_dir: Option<String>) -> Self {
        FilesystemImageStore {
            root_dir: root_dir.map_or_else(|| "examples/ebay-clone/backend/data/images".into(), PathBuf::from),
        }
    }

    fn file_path(&self, sha256: &str) -> PathBuf {
        let ext = match self.root_dir.join(format!("{}.png", sha256)).exists() {
            true => "png",
            false if self.root_dir.join(format!("{}.jpg", sha256)).exists() || self.root_dir.join(format!("{}.jpeg", sha256)).exists() => "jpg",
            _ => "",
        };
        self.root_dir.join(format!("{}.{}", sha256, ext))
    }
}

// Conforms to `ImageStorePort` exactly: `store_image(Vec<u8>) -> Result<String,
// DomainError>` and `get_image(&str) -> Result<Vec<u8>, DomainError>`. The port
// is the contract; the previous impl invented a `content_type` arg, an
// `ImageRef` return, and an `ImageStoreError` type that the port never declared.
#[async_trait::async_trait]
impl ImageStorePort for FilesystemImageStore {
    async fn store_image(&self, image_data: Vec<u8>) -> Result<String, DomainError> {
        if image_data.len() > MAX_FILE_SIZE {
            return Err(DomainError::StorageError("image exceeds maximum allowed size".to_string()));
        }

        let sha256 = hasher::compute_sha256(&image_data);
        let file_path = self.file_path(&sha256);

        let mut file = File::create(&file_path)
            .await
            .map_err(|e| DomainError::StorageError(e.to_string()))?;
        file.write_all(&image_data)
            .await
            .map_err(|e| DomainError::StorageError(e.to_string()))?;

        Ok(sha256)
    }

    async fn get_image(&self, hash: &str) -> Result<Vec<u8>, DomainError> {
        let file_path = self.file_path(hash);
        if !file_path.exists() {
            return Err(DomainError::StorageError("image not found".to_string()));
        }

        let data = fs::read(&file_path).map_err(|e| DomainError::StorageError(e.to_string()))?;
        Ok(data)
    }
}