use crate::core::domain::StoreError;
use crate::core::ports::IStorePort;

pub struct HttpAdapter<S: IStorePort> {
    service: S,
}

impl<S: IStorePort> HttpAdapter<S> {
    pub fn new(service: S) -> Self {
        Self { service }
    }

    pub fn handle_get(&self, key: &str) -> Result<String, StoreError> {
        let kv = self.service.get(key)?;
        Ok(format!("{}={}", kv.key, kv.value))
    }

    pub fn handle_set(&self, key: &str, value: &str) -> Result<String, StoreError> {
        self.service.set(key, value)?;
        Ok(format!("OK: {} set", key))
    }

    pub fn handle_delete(&self, key: &str) -> Result<String, StoreError> {
        self.service.delete(key)?;
        Ok(format!("OK: {} deleted", key))
    }

    pub fn handle_list(&self) -> Result<String, StoreError> {
        let items = self.service.list()?;
        let lines: Vec<String> = items
            .iter()
            .map(|kv| format!("{}={}", kv.key, kv.value))
            .collect();
        Ok(lines.join("\n"))
    }
}
