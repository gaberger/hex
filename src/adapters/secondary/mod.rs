// src/adapters/secondary/mod.rs

pub mod image_store_fs {
    pub use super::image_store_fs::{ImageStoreFs, new as image_store_new, store_image, retrieve_image};
}

pub mod password_hasher_argon2 {
    pub use super::password_hasher_argon2::{PasswordHasherArgon2, new as password_hasher_new, hash_password};
}

pub mod stdb_client {
    pub use super::stdb_client::{StdBClient, new as stdb_client_new, execute_query, analyze_data};
}