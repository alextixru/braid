//! Хранилище ядра: последняя подписка и `etag`, свои правила, настройки,
//! память сети, флаг «сессия была активна». Ключ → байты; секреты устройства
//! (токен VK) живут в защищённом хранилище на стороне приложения, не здесь.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StorageError {
    #[error("недопустимый ключ хранилища «{0}»")]
    BadKey(String),
    #[error("хранилище: {0}")]
    Io(String),
}

pub trait Storage: Send + Sync {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError>;
    fn write(&self, key: &str, data: &[u8]) -> Result<(), StorageError>;
    fn remove(&self, key: &str) -> Result<(), StorageError>;
    /// Каталог для файлов, которые не помещаются в ключи (журнал JSONL);
    /// `None` у хранилища в памяти.
    fn dir(&self) -> Option<PathBuf>;
}

/// Файл на ключ в каталоге приложения; запись атомарная (tmp + rename).
#[derive(Debug, Clone)]
pub struct FsStorage {
    dir: PathBuf,
}

impl FsStorage {
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let dir = dir.into();
        fs::create_dir_all(&dir)
            .map_err(|e| StorageError::Io(format!("{}: {e}", dir.display())))?;
        Ok(Self { dir })
    }

    fn path(&self, key: &str) -> Result<PathBuf, StorageError> {
        let ok = !key.is_empty()
            && !key.starts_with('.')
            && key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.');
        if ok {
            Ok(self.dir.join(key))
        } else {
            Err(StorageError::BadKey(key.to_owned()))
        }
    }
}

fn io_err(path: &Path, e: std::io::Error) -> StorageError {
    StorageError::Io(format!("{}: {e}", path.display()))
}

impl Storage for FsStorage {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let path = self.path(key)?;
        match fs::read(&path) {
            Ok(data) => Ok(Some(data)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(&path, e)),
        }
    }

    fn write(&self, key: &str, data: &[u8]) -> Result<(), StorageError> {
        let path = self.path(key)?;
        let tmp = self.dir.join(format!(".{key}.tmp"));
        let result = (|| {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(data)?;
            f.sync_all()?;
            fs::rename(&tmp, &path)
        })();
        result.map_err(|e| {
            let _ = fs::remove_file(&tmp);
            io_err(&path, e)
        })
    }

    fn remove(&self, key: &str) -> Result<(), StorageError> {
        let path = self.path(key)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_err(&path, e)),
        }
    }

    fn dir(&self) -> Option<PathBuf> {
        Some(self.dir.clone())
    }
}
