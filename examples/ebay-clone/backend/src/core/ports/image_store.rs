use async_trait::async_trait;
use crate::core::domain::*;

/// ImageStorePort trait for storing and retrieving images by SHA256 hash.
///
/// # Specifications
/// - docs/specs/ebay-spec-019
///
/// This trait defines the interface for an image storage system that can store images asynchronously and retrieve them using their SHA256 hashes. It is designed to be agnostic of the underlying storage mechanism, ensuring that adapters do not leak domain types they shouldn't see.
#[async_trait]
pub trait ImageStorePort: Send + Sync {
    /// Stores an image in the image storage system.
    ///
    /// # Arguments
    /// * `image_data` - A byte vector containing the image data to be stored.
    ///
    /// # Returns
    /// * The SHA256 hash of the stored image as a String, or an error if the operation fails.
    async fn store_image(&self, image_data: Vec<u8>) -> Result<String, DomainError>;

    /// Retrieves an image from the image storage system by its SHA256 hash.
    ///
    /// # Arguments
    /// * `hash` - The SHA256 hash of the image to be retrieved as a String.
    ///
    /// # Returns
    /// * A byte vector containing the image data, or an error if the operation fails.
    async fn get_image(&self, hash: &str) -> Result<Vec<u8>, DomainError>;
}