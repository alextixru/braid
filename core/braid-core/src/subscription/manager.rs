//! Менеджер подписки: импорт ссылки и замена после подтверждения (ист. 35,
//! 76), обновление при подключении и раз в `update_interval_hours` (ист. 37),
//! сбои без потери рабочей копии (ист. 38, 39), отзыв (ист. 47), баннер
//! новой версии (ист. 73). Сеть — через [`Fetcher`]; время — через [`Clock`].
//!
//! Что меняется после `Updated`, отдаётся в [`RefreshReport::changes`]:
//! правила — ядру применять сразу (`RulesChange::FromSubscription`), движки
//! с новыми параметрами — помечаются и ждут следующего подключения движка
//! ([`SubscriptionManager::take_pending_engines`]).

use super::store::{Meta, RefreshFailure, Store, StoreError};
use super::{FetchError, FetchOutcome, Fetcher};
use crate::clock::Clock;
use crate::state::{Banner, RefreshOutcome, RulesChange, SubscriptionView};
use crate::storage::Storage;
use braid_proto::{BraidLink, EngineId, LinkError, Subscription, SubscriptionError};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefreshError {
    #[error("Подписки нет — сначала вставь ссылку или отсканируй QR")]
    NoSubscription,
    #[error("{0}")]
    Fetch(#[from] FetchError),
    #[error("{0}")]
    Subscription(#[from] SubscriptionError),
    #[error("Доступ отозван: хост больше не отдаёт эту подписку")]
    Revoked,
    #[error("{0}")]
    Storage(#[from] StoreError),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("{0}")]
    Link(#[from] LinkError),
    #[error("{0}")]
    Refresh(#[from] RefreshError),
}

/// Что изменилось в подписке после `Updated`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionChange {
    /// Наборы или режимы другие — применить сейчас.
    Rules { change: RulesChange },
    /// Параметры этих движков другие — при следующем подключении движка.
    Engines { ids: Vec<EngineId> },
    /// Свежая версия клиента в подписке.
    Client { latest: String },
}

/// Итог попытки обновления: результат, дата последнего успеха (этой попытки
/// или прежняя) и список изменений при `Updated`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshReport {
    pub outcome: Result<RefreshOutcome, RefreshError>,
    pub last_success: Option<DateTime<Utc>>,
    pub changes: Vec<SubscriptionChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportOutcome {
    Imported(RefreshReport),
    /// Подписка уже есть, а ссылка другая: замена — только после подтверждения
    /// (`replace`). `new_host` — хост из новой ссылки.
    ReplaceRequested {
        current_host: String,
        new_host: String,
    },
}

#[derive(Default)]
struct Inner {
    link: Option<BraidLink>,
    current: Option<Subscription>,
    meta: Meta,
    /// Сохранённая копия не читается — до первого успешного обновления.
    corrupt: Option<String>,
}

pub struct SubscriptionManager {
    store: Store,
    clock: Arc<dyn Clock>,
    fetcher: Arc<dyn Fetcher>,
    app_version: String,
    platform: String,
    inner: Mutex<Inner>,
}

fn default_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "other"
    }
}

impl SubscriptionManager {
    /// Поднять состояние из хранилища. Битая копия не мешает запуску: ссылка
    /// остаётся, ошибка видна, первое обновление чинит.
    pub fn new(
        storage: Arc<dyn Storage>,
        clock: Arc<dyn Clock>,
        fetcher: Arc<dyn Fetcher>,
    ) -> Self {
        let store = Store::new(storage);
        let inner = match store.load() {
            Ok(Some(stored)) => Inner {
                link: Some(stored.link),
                current: Some(stored.subscription),
                meta: stored.meta,
                corrupt: None,
            },
            Ok(None) => Inner::default(),
            Err(e) => {
                tracing::warn!(error = %e, "сохранённая подписка не прочитана");
                Inner {
                    link: store.link().ok().flatten(),
                    current: None,
                    meta: store.meta().unwrap_or_default(),
                    corrupt: Some(e.to_string()),
                }
            }
        };
        Self {
            store,
            clock,
            fetcher,
            app_version: crate::VERSION.to_owned(),
            platform: default_platform().to_owned(),
            inner: Mutex::new(inner),
        }
    }

    /// Версия приложения для баннера «Доступно обновление» (по умолчанию — ядра).
    pub fn with_app_version(mut self, version: impl Into<String>) -> Self {
        self.app_version = version.into();
        self
    }

    /// Платформа для ссылки на сборку: `macos`, `android`.
    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = platform.into();
        self
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Последняя рабочая копия.
    pub fn current(&self) -> Option<Subscription> {
        self.lock().current.clone()
    }

    pub fn link(&self) -> Option<BraidLink> {
        self.lock().link.clone()
    }

    pub fn meta(&self) -> Meta {
        self.lock().meta.clone()
    }

    /// `braid://…`: первая ссылка — загрузка; та же — обновление; другая при
    /// живой подписке — [`ImportOutcome::ReplaceRequested`], сеть не трогается.
    pub fn import(&self, link: &str) -> Result<ImportOutcome, ImportError> {
        let link = BraidLink::parse(link)?;
        let existing = {
            let inner = self.lock();
            inner
                .link
                .clone()
                .map(|l| (l, inner.current.as_ref().map(|s| s.host.name.clone())))
        };
        match existing {
            Some((current_link, host_name)) if current_link != link => {
                Ok(ImportOutcome::ReplaceRequested {
                    current_host: host_name.unwrap_or_else(|| url_host(&current_link)),
                    new_host: url_host(&link),
                })
            }
            Some(_) => Ok(ImportOutcome::Imported(self.refresh())),
            None => Ok(ImportOutcome::Imported(self.adopt(link)?)),
        }
    }

    /// Замена подписки целиком после подтверждения пользователя; при отказе
    /// хоста старая остаётся. Свои правила живут в других ключах и не трогаются.
    pub fn replace(&self, link: &str) -> Result<RefreshReport, ImportError> {
        let link = BraidLink::parse(link)?;
        Ok(self.adopt(link)?)
    }

    /// Загрузить подписку по новой ссылке и принять её; до успеха состояние не
    /// меняется. Изменения — относительно прежней копии (при замене хоста),
    /// при первом импорте перезапускать нечего: отметок движкам нет.
    fn adopt(&self, link: BraidLink) -> Result<RefreshReport, RefreshError> {
        let previous = self.lock().current.clone();
        let fetched = self.fetcher.fetch(&link, None);
        let now = self.clock.utc_now();
        let (body, etag) = match fetched {
            Ok(FetchOutcome::Fetched { body, etag }) => (body, etag),
            Ok(FetchOutcome::NotModified) => {
                return Err(FetchError::Invalid("304 на запрос без ETag".into()).into());
            }
            Ok(FetchOutcome::Revoked) => return Err(RefreshError::Revoked),
            Err(e) => return Err(e.into()),
        };
        let subscription = Subscription::from_json(&body)?;
        let (changes, changed_engines) = diff(previous.as_ref(), &subscription);
        let meta = Meta {
            etag,
            fetched_at: Some(now),
            last_attempt_at: Some(now),
            last_error: None,
            revoked: false,
            pending_engines: if previous.is_some() {
                changed_engines
            } else {
                Vec::new()
            },
        };
        self.store.save(&link, &body, &meta)?;
        let version = subscription.version;
        let mut inner = self.lock();
        *inner = Inner {
            link: Some(link),
            current: Some(subscription),
            meta,
            corrupt: None,
        };
        Ok(RefreshReport {
            outcome: Ok(RefreshOutcome::Updated { version }),
            last_success: Some(now),
            changes,
        })
    }

    /// «Обновить сейчас»: запрос с `ETag`, сбой не трогает рабочую копию.
    pub fn refresh(&self) -> RefreshReport {
        let (link, etag, previous) = {
            let inner = self.lock();
            let Some(link) = inner.link.clone() else {
                return RefreshReport {
                    outcome: Err(RefreshError::NoSubscription),
                    last_success: None,
                    changes: Vec::new(),
                };
            };
            // Без рабочей копии etag не шлём: 304 было бы не к чему применить.
            let etag = inner.current.as_ref().and_then(|_| inner.meta.etag.clone());
            (link, etag, inner.current.clone())
        };
        let fetched = self.fetcher.fetch(&link, etag.as_deref());
        let now = self.clock.utc_now();

        let mut inner = self.lock();
        if inner.link.as_ref() != Some(&link) {
            return RefreshReport {
                outcome: Err(
                    FetchError::Invalid("ссылка сменилась во время обновления".into()).into(),
                ),
                last_success: inner.meta.fetched_at,
                changes: Vec::new(),
            };
        }
        inner.meta.last_attempt_at = Some(now);
        let result = match fetched {
            Ok(FetchOutcome::Fetched { body, etag }) => match Subscription::from_json(&body) {
                Ok(subscription) => {
                    let (changes, changed_engines) = diff(previous.as_ref(), &subscription);
                    let mut meta = inner.meta.clone();
                    meta.etag = etag;
                    meta.fetched_at = Some(now);
                    meta.last_error = None;
                    meta.revoked = false;
                    for id in changed_engines {
                        if !meta.pending_engines.contains(&id) {
                            meta.pending_engines.push(id);
                        }
                    }
                    match self.store.save(&link, &body, &meta) {
                        Ok(()) => {
                            let version = subscription.version;
                            inner.meta = meta;
                            inner.current = Some(subscription);
                            inner.corrupt = None;
                            Ok((RefreshOutcome::Updated { version }, changes))
                        }
                        Err(e) => Err(RefreshError::Storage(e)),
                    }
                }
                Err(e) => Err(RefreshError::Subscription(e)),
            },
            Ok(FetchOutcome::NotModified) if previous.is_some() => {
                inner.meta.fetched_at = Some(now);
                inner.meta.last_error = None;
                inner.meta.revoked = false;
                Ok((RefreshOutcome::NotModified, Vec::new()))
            }
            Ok(FetchOutcome::NotModified) => {
                Err(FetchError::Invalid("304 на запрос без ETag".into()).into())
            }
            Ok(FetchOutcome::Revoked) => {
                inner.meta.revoked = true;
                Err(RefreshError::Revoked)
            }
            Err(e) => Err(RefreshError::Fetch(e)),
        };
        let (outcome, changes) = match result {
            Ok((outcome, changes)) => (Ok(outcome), changes),
            Err(e) => {
                inner.meta.last_error = Some(RefreshFailure {
                    at: now,
                    message: e.to_string(),
                });
                (Err(e), Vec::new())
            }
        };
        if let Err(e) = self.store.save_meta(&inner.meta) {
            tracing::warn!(error = %e, "мета подписки не записана");
        }
        RefreshReport {
            outcome,
            last_success: inner.meta.fetched_at,
            changes,
        }
    }

    /// При каждом подключении — обновить, если подписка есть.
    pub fn on_connect(&self) -> Option<RefreshReport> {
        self.lock().link.as_ref()?;
        Some(self.refresh())
    }

    /// Пора ли обновлять по расписанию: прошло `update_interval_hours` с последней
    /// попытки; `0` — только при подключении; битая копия — сразу.
    pub fn is_due(&self) -> bool {
        let inner = self.lock();
        if inner.link.is_none() {
            return false;
        }
        let Some(current) = &inner.current else {
            return true;
        };
        let hours = current.update_interval_hours;
        if hours == 0 {
            return false;
        }
        match inner.meta.last_attempt_at {
            None => true,
            Some(last) => self.clock.utc_now() - last >= ChronoDuration::hours(i64::from(hours)),
        }
    }

    /// Тик планировщика: обновление, если пора.
    pub fn tick(&self) -> Option<RefreshReport> {
        self.is_due().then(|| self.refresh())
    }

    /// Движки, чьи параметры изменились и ещё не применены.
    pub fn pending_engines(&self) -> Vec<EngineId> {
        self.lock().meta.pending_engines.clone()
    }

    /// Забрать и снять отметку — когда движок перезапускается с новыми параметрами.
    pub fn take_pending_engines(&self) -> Vec<EngineId> {
        let mut inner = self.lock();
        let ids = std::mem::take(&mut inner.meta.pending_engines);
        if !ids.is_empty()
            && let Err(e) = self.store.save_meta(&inner.meta)
        {
            tracing::warn!(error = %e, "мета подписки не записана");
        }
        ids
    }

    /// Вид для экрана «Подписка».
    pub fn view(&self) -> SubscriptionView {
        let inner = self.lock();
        let current = inner.current.as_ref();
        SubscriptionView {
            host_id: current.map(|s| s.host.id.clone()),
            host_name: current.map(|s| s.host.name.clone()),
            version: current.map(|s| s.version),
            updated_at: current.map(|s| s.updated_at),
            last_refresh_error: inner
                .meta
                .last_error
                .as_ref()
                .map(|e| e.message.clone())
                .or_else(|| inner.corrupt.clone()),
            revoked: inner.meta.revoked,
        }
    }

    /// Баннеры: отзыв, новая версия клиента (ист. 73), «не обновлялась с …» (ист. 38).
    pub fn banners(&self) -> Vec<Banner> {
        let inner = self.lock();
        let mut banners = Vec::new();
        if inner.meta.revoked {
            banners.push(Banner::Revoked);
        }
        if let Some(client) = inner.current.as_ref().and_then(|s| s.client.as_ref())
            && is_newer(&client.latest, &self.app_version)
        {
            let url = client
                .downloads
                .get(&self.platform)
                .or_else(|| client.downloads.values().next())
                .map(|u| u.to_string());
            banners.push(Banner::UpdateAvailable {
                latest: client.latest.clone(),
                url,
            });
        }
        if inner.meta.last_error.is_some()
            && let Some(since) = inner.meta.fetched_at
        {
            banners.push(Banner::SubscriptionStale { since });
        }
        banners
    }
}

fn url_host(link: &BraidLink) -> String {
    link.url.host_str().unwrap_or_default().to_owned()
}

/// Изменения между копиями и список движков с другими параметрами
/// (добавленные, изменённые, удалённые).
fn diff(
    previous: Option<&Subscription>,
    next: &Subscription,
) -> (Vec<SubscriptionChange>, Vec<EngineId>) {
    let mut changes = Vec::new();
    if previous.is_none_or(|p| p.rules != next.rules) {
        changes.push(SubscriptionChange::Rules {
            change: RulesChange::FromSubscription,
        });
    }
    let mut engines: Vec<EngineId> = next
        .engines
        .iter()
        .filter(|e| previous.and_then(|p| p.engine(&e.id())) != Some(*e))
        .map(|e| e.id())
        .collect();
    if let Some(p) = previous {
        for e in &p.engines {
            if next.engine(&e.id()).is_none() {
                engines.push(e.id());
            }
        }
    }
    if !engines.is_empty() {
        changes.push(SubscriptionChange::Engines {
            ids: engines.clone(),
        });
    }
    let latest = next.client.as_ref().map(|c| c.latest.as_str());
    if latest.is_some()
        && latest
            != previous
                .and_then(|p| p.client.as_ref())
                .map(|c| c.latest.as_str())
    {
        changes.push(SubscriptionChange::Client {
            latest: latest.unwrap_or_default().to_owned(),
        });
    }
    (changes, engines)
}

/// Числовые компоненты версии: `v0.2.0-dev+5` → `[0, 2, 0]`.
fn version_key(v: &str) -> Vec<u64> {
    let core = v.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next().unwrap_or("");
    core.split('.')
        .map(|p| p.trim().parse::<u64>().unwrap_or(0))
        .collect()
}

/// `latest` строго новее `app` по числовым компонентам.
fn is_newer(latest: &str, app: &str) -> bool {
    let (a, b) = (version_key(latest), version_key(app));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}
