use crate::core::domain::{KeyValue, StoreError};

pub trait IStorePort: Send + Sync {
    fn get(&self, key: &str) -> Result<KeyValue, StoreError>;
    fn set(&self, key: &str, value: &str) -> Result<(), StoreError>;
    fn delete(&self, key: &str) -> Result<(), StoreError>;
    fn list(&self) -> Result<Vec<KeyValue>, StoreError>;
}

pub trait ISerializerPort: Send + Sync {
    fn serialize(&self, kv: &KeyValue) -> Result<String, StoreError>;
    fn deserialize(&self, data: &str) -> Result<KeyValue, StoreError>;
}
