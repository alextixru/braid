//! Смена сети на десктопе. Android сообщает о ней сам (`NetworkCallback` →
//! JNI); на macOS платформа лишь отвечает на `network_id()`, и о смене ядру
//! надо сказать: движки переоткрывают сокеты (ист. 27), а правила «по сети»
//! подключают или спрашивают (ист. 58). Здесь — опрос раз в несколько секунд.

use braid_core::proto::{NetworkId, NetworkKind};
use braid_core::{Core, Platform};
use std::sync::Arc;
use std::time::Duration;

/// Как часто спрашивать у платформы идентификатор сети.
pub const POLL: Duration = Duration::from_secs(5);

/// Помнит последнюю сеть и отдаёт только изменения.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetWatch {
    last: Option<NetworkId>,
}

impl NetWatch {
    /// `initial` — то, что ядро уже знает при создании: повторно его не сообщаем.
    pub fn new(initial: Option<&NetworkId>) -> Self {
        Self {
            last: initial.cloned(),
        }
    }

    /// Новая сеть относительно прошлого опроса. «Сети нет» — не изменение:
    /// вернувшись в ту же сеть, сообщать нечего, в другую — сообщим.
    pub fn changed(
        &mut self,
        now: Option<(NetworkId, NetworkKind)>,
    ) -> Option<(NetworkId, NetworkKind)> {
        let (id, kind) = now?;
        if self.last.as_ref() == Some(&id) {
            return None;
        }
        self.last = Some(id.clone());
        Some((id, kind))
    }
}

/// Поток опроса: пока живо ядро процесса, сообщает ему о смене сети.
pub fn spawn(platform: Arc<dyn Platform>, core: &'static Core) {
    let mut watch = NetWatch::new(platform.network_id().as_ref().map(|(id, _)| id));
    let _ = std::thread::Builder::new()
        .name("braid-netwatch".into())
        .spawn(move || loop {
            std::thread::sleep(POLL);
            if let Some((id, kind)) = watch.changed(platform.network_id()) {
                core.on_network_changed(id, kind);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(id: &str) -> Option<(NetworkId, NetworkKind)> {
        Some((NetworkId::new(id), NetworkKind::Wifi))
    }

    #[test]
    fn the_network_the_core_already_knows_is_not_reported() {
        let known = NetworkId::new("wifi:a");
        let mut watch = NetWatch::new(Some(&known));
        assert_eq!(watch.changed(net("wifi:a")), None);
    }

    #[test]
    fn a_new_network_is_reported_once() {
        let known = NetworkId::new("wifi:a");
        let mut watch = NetWatch::new(Some(&known));
        assert_eq!(watch.changed(net("wifi:b")), net("wifi:b"));
        assert_eq!(watch.changed(net("wifi:b")), None);
        assert_eq!(watch.changed(net("wifi:a")), net("wifi:a"));
    }

    #[test]
    fn losing_the_network_is_silent_and_coming_back_to_the_same_one_too() {
        let known = NetworkId::new("wifi:a");
        let mut watch = NetWatch::new(Some(&known));
        assert_eq!(watch.changed(None), None);
        assert_eq!(watch.changed(net("wifi:a")), None);
        assert_eq!(watch.changed(None), None);
        assert_eq!(watch.changed(net("gw:c")), net("gw:c"));
    }

    #[test]
    fn the_first_network_after_starting_offline_is_reported() {
        let mut watch = NetWatch::new(None);
        assert_eq!(watch.changed(None), None);
        assert_eq!(watch.changed(net("wifi:a")), net("wifi:a"));
    }
}
