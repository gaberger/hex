#[derive(Clone)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
    pub created_at: u64,
}

pub struct StoreError {
    pub message: String,
    pub kind: ErrorKind,
}

pub enum ErrorKind {
    NotFound,
    AlreadyExists,
    Internal,
}
