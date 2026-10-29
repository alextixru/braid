//! `MemoryStorage`: хранилище в памяти, переживает «перезапуск ядра» в тесте,
//! если держать его в `Arc`.

use crate::storage::{Storage, StorageError};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Default)]
pub struct MemoryStorage {
    map: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn keys(&self) -> Vec<String> {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }
}

impl Storage for MemoryStorage {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(self
            .map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned())
    }

    fn write(&self, key: &str, data: &[u8]) -> Result<(), StorageError> {
        if key.is_empty() {
            return Err(StorageError::BadKey(key.to_owned()));
        }
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_owned(), data.to_vec());
        Ok(())
    }

    fn remove(&self, key: &str) -> Result<(), StorageError> {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
        Ok(())
    }

    fn dir(&self) -> Option<PathBuf> {
        None
    }
}
