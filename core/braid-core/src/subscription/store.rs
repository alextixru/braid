//! Последняя рабочая копия подписки в [`Storage`]: ссылка, текст подписки
//! как пришёл с хоста (его и сверяет `ETag`) и мета — `etag`, даты успеха и
//! попытки, ошибка последней попытки, отзыв, движки с новыми адресами.
//!
//! Три ключа, ничего чужого: свои правила и переключатели модуля правил
//! при замене подписки не трогаются (ист. 76). Битый файл на диске —
//! [`StoreError::Corrupt`]: ссылка остаётся, копию чинит следующее обновление.

use crate::storage::{Storage, StorageError};
use braid_proto::{BraidLink, EngineId, Subscription};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

pub const LINK_KEY: &str = "subscription_link.json";
pub const BODY_KEY: &str = "subscription.json";
pub const META_KEY: &str = "subscription_meta.json";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("{0}")]
    Storage(#[from] StorageError),
    #[error("Сохранённая подписка повреждена: {0}")]
    Corrupt(String),
}

/// Чем кончилась последняя неудачная попытка — текст для экрана (ист. 38).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshFailure {
    pub at: DateTime<Utc>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub etag: Option<String>,
    /// Последний успех: `200` или `304`.
    #[serde(default)]
    pub fetched_at: Option<DateTime<Utc>>,
    /// Последняя попытка, удачная или нет — от неё считается расписание.
    #[serde(default)]
    pub last_attempt_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_error: Option<RefreshFailure>,
    /// Хост ответил `404`: доступ отозван, копия оставлена для экрана.
    #[serde(default)]
    pub revoked: bool,
    /// Движки, чьи параметры изменились: применятся при следующем подключении движка.
    #[serde(default)]
    pub pending_engines: Vec<EngineId>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Stored {
    pub link: BraidLink,
    pub subscription: Subscription,
    /// Текст с хоста байт в байт.
    pub body: String,
    pub meta: Meta,
}

impl fmt::Debug for Stored {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stored")
            .field("link", &self.link)
            .field("subscription", &self.subscription)
            .field("body", &"[скрыто]")
            .field("meta", &self.meta)
            .finish()
    }
}

pub struct Store {
    storage: Arc<dyn Storage>,
}

impl Store {
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self { storage }
    }

    pub fn link(&self) -> Result<Option<BraidLink>, StoreError> {
        match self.storage.read(LINK_KEY)? {
            None => Ok(None),
            Some(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| StoreError::Corrupt(format!("ссылка: {e}"))),
        }
    }

    /// Мета; нечитаемая — как отсутствующая (без `etag` следующий запрос
    /// просто получит полное тело).
    pub fn meta(&self) -> Result<Meta, StoreError> {
        Ok(match self.storage.read(META_KEY)? {
            None => Meta::default(),
            Some(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "мета подписки не читается — сброшена");
                Meta::default()
            }),
        })
    }

    /// `None` — ссылки нет; `Corrupt` — ссылка есть, а копия не читается.
    pub fn load(&self) -> Result<Option<Stored>, StoreError> {
        let Some(link) = self.link()? else {
            return Ok(None);
        };
        let body = self
            .storage
            .read(BODY_KEY)?
            .ok_or_else(|| StoreError::Corrupt("нет файла подписки".into()))?;
        let body = String::from_utf8(body).map_err(|_| StoreError::Corrupt("не UTF-8".into()))?;
        let subscription =
            Subscription::from_json(&body).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        let meta = self.meta()?;
        Ok(Some(Stored {
            link,
            subscription,
            body,
            meta,
        }))
    }

    /// Тело, мета, затем ссылка — чтобы оборванная запись не оставила ссылку
    /// без тела дольше необходимого.
    pub fn save(&self, link: &BraidLink, body: &str, meta: &Meta) -> Result<(), StoreError> {
        self.storage.write(BODY_KEY, body.as_bytes())?;
        self.save_meta(meta)?;
        let link = serde_json::to_vec(link).expect("ссылка всегда сериализуется");
        self.storage.write(LINK_KEY, &link)?;
        Ok(())
    }

    pub fn save_meta(&self, meta: &Meta) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec(meta).expect("мета всегда сериализуется");
        self.storage.write(META_KEY, &bytes)?;
        Ok(())
    }

    /// Убрать только свои ключи.
    pub fn clear(&self) -> Result<(), StoreError> {
        for key in [LINK_KEY, BODY_KEY, META_KEY] {
            self.storage.remove(key)?;
        }
        Ok(())
    }
}
