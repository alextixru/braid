//! `ScriptedFetcher` — подделка [`Fetcher`] для тестов менеджера и ядра:
//! отдаёт заранее заданные ответы по очереди и запоминает, с чем её звали.

use super::{FetchError, FetchOutcome, Fetcher};
use braid_proto::BraidLink;
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptedCall {
    pub link: BraidLink,
    pub etag: Option<String>,
}

#[derive(Default)]
pub struct ScriptedFetcher {
    queue: Mutex<VecDeque<Result<FetchOutcome, FetchError>>>,
    calls: Mutex<Vec<ScriptedCall>>,
}

impl ScriptedFetcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Следующий ответ; пустая очередь отвечает «хост недоступен».
    pub fn push(&self, response: Result<FetchOutcome, FetchError>) {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(response);
    }

    pub fn calls(&self) -> Vec<ScriptedCall> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Fetcher for ScriptedFetcher {
    fn fetch(&self, link: &BraidLink, etag: Option<&str>) -> Result<FetchOutcome, FetchError> {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(ScriptedCall {
                link: link.clone(),
                etag: etag.map(str::to_owned),
            });
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
            .unwrap_or_else(|| Err(FetchError::Unreachable("сценарий пуст".into())))
    }
}
