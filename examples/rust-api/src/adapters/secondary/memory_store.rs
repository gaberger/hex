use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::domain::{ErrorKind, KeyValue, StoreError};
use crate::core::ports::IStorePort;

pub struct MemoryStoreAdapter {
    data: RwLock<HashMap<String, KeyValue>>,
}

impl MemoryStoreAdapter {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
        }
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

impl IStorePort for MemoryStoreAdapter {
    fn get(&self, key: &str) -> Result<KeyValue, StoreError> {
        let data = self.data.read().map_err(|_| StoreError {
            message: "Lock poisoned".into(),
            kind: ErrorKind::Internal,
        })?;
        data.get(key).cloned().ok_or(StoreError {
            message: format!("Key '{}' not found", key),
            kind: ErrorKind::NotFound,
        })
    }

    fn set(&self, key: &str, value: &str) -> Result<(), StoreError> {
        let mut data = self.data.write().map_err(|_| StoreError {
            message: "Lock poisoned".into(),
            kind: ErrorKind::Internal,
        })?;
        data.insert(
            key.to_string(),
            KeyValue {
                key: key.to_string(),
                value: value.to_string(),
                created_at: Self::now(),
            },
        );
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), StoreError> {
        let mut data = self.data.write().map_err(|_| StoreError {
            message: "Lock poisoned".into(),
            kind: ErrorKind::Internal,
        })?;
        data.remove(key).ok_or(StoreError {
            message: format!("Key '{}' not found", key),
            kind: ErrorKind::NotFound,
        })?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<KeyValue>, StoreError> {
        let data = self.data.read().map_err(|_| StoreError {
            message: "Lock poisoned".into(),
            kind: ErrorKind::Internal,
        })?;
        Ok(data.values().cloned().collect())
    }
}
