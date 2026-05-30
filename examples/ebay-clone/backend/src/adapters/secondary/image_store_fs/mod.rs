use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use sha2::{Sha256, Digest};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::core::domain::image_store::{ImageRef, ImageStoreError, ImageStorePort};

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
    async fn store_image(&self, image_data: Vec<u8>, content_type: &str) -> Result<ImageRef, ImageStoreError> {
        if !SUPPORTED_CONTENT_TYPES.contains(&content_type) {
            return Err(ImageStoreError::UnsupportedContentType);
        }

        let sha256 = hasher::compute_sha256(&image_data);
        let file_path = self.file_path(&sha256);

        let mut file = File::create(&file_path).await.map_err(|_| ImageStoreError::Io)?;
        file.write_all(&image_data).await.map_err(|_| ImageStoreError::Io)?;

        Ok(ImageRef { sha256 })
    }

    async fn get_image(&self, image_ref: &ImageRef) -> Result<Vec<u8>, ImageStoreError> {
        let file_path = self.file_path(&image_ref.sha256);
        if !file_path.exists() {
            return Err(ImageStoreError::NotFound);
        }

        let data = fs::read(&file_path).map_err(|_| ImageStoreError::Io)?;
        Ok(data)
    }
}