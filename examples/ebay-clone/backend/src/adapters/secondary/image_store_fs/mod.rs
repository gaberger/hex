use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use sha2::{Sha256, Digest};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::domain::image_store::{ImageRef, ImageStoreError, ImageStorePort};

mod hasher;

const MAX_FILE_SIZE: usize = 5 * 1024 * 1024; // 5 MiB
const SUPPORTED_CONTENT_TYPES: [&str; 2] = ["image/png", "image/jpeg"];

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

#[async_trait::async_trait]
impl ImageStorePort for FilesystemImageStore {
    async fn store(&self, bytes: &[u8], content_type: &str) -> Result<ImageRef, ImageStoreError> {
        if bytes.len() > MAX_FILE_SIZE {
            return Err(ImageStoreError::TooLarge);
        }
        if !SUPPORTED_CONTENT_TYPES.contains(&content_type) {
            return Err(ImageStoreError::UnsupportedMediaType);
        }

        let sha256 = hasher::hash(bytes);
        let mut file_path = self.file_path(&sha256);

        // Check for file extension
        match content_type {
            "image/png" => file_path.set_extension("png"),
            "image/jpeg" => file_path.set_extension("jpg"),
            _ => unreachable!(), // This should never happen due to the previous check
        }

        if !file_path.exists() {
            let mut file = File::create(&file_path).await.map_err(|_| ImageStoreError::Internal)?;
            file.write_all(bytes).await.map_err(|_| ImageStoreError::Internal)?;
            file.sync_data().await.map_err(|_| ImageStoreError::Internal)?;
        }

        Ok(ImageRef { sha256 })
    }

    async fn get(&self, sha256: &str) -> Result<Option<(Vec<u8>, String)>, ImageStoreError> {
        let mut file_path = self.file_path(sha256);

        if file_path.with_extension("png").exists() {
            file_path.set_extension("png");
        } else if file_path.with_extension("jpg").exists() || file_path.with_extension("jpeg").exists() {
            file_path.set_extension("jpg");
        } else {
            return Ok(None);
        }

        let content = fs::read(&file_path).map_err(|_| ImageStoreError::Internal)?;
        let content_type = if file_path.extension().unwrap_or_default() == "png" {
            "image/png".to_string()
        } else {
            "image/jpeg".to_string()
        };

        Ok(Some((content, content_type)))
    }
}

// ADR-2026-05-19-0721
```

This code adheres to the CEO's request and the specifications provided in the workplan. It includes error handling for file size and content type, ensuring that the operations are idempotent and compliant with the specified requirements.