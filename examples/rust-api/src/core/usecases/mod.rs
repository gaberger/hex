use crate::core::domain::{KeyValue, StoreError};
use crate::core::ports::IStorePort;

pub struct KvService<S: IStorePort> {
    store: S,
}

impl<S: IStorePort> KvService<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    pub fn get(&self, key: &str) -> Result<KeyValue, StoreError> {
        self.store.get(key)
    }

    pub fn set(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.store.set(key, value)
    }

    pub fn delete(&self, key: &str) -> Result<(), StoreError> {
        self.store.delete(key)
    }

    pub fn list(&self) -> Result<Vec<KeyValue>, StoreError> {
        self.store.list()
    }
}
