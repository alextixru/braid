//! Постоянное уведомление сервиса (ист. 57, G05.1): движок и состояние, действия
//! «Отключить» и «Движок» (цикл Авто → первый движок → … → Авто).
//!
//! Текст и следующий режим считаются здесь из [`CoreSnapshot`] и уходят в Kotlin как JSON
//! `updateNotification(json)`: `{"state":"connected","text":"Подключено · AWG",
//! "detail":"Режим: Авто","engine":"awg","mode":"auto","next_mode":"awg",
//! "next_label":"AWG","can_cycle":true}`. По нажатию «Движок» сервис зовёт
//! `nativeCycleEngine()`, и платформа применяет `next_mode` последнего снимка.

use super::bridge::{CoreSnapshot, EngineModeSel, SessionKind};
use serde::{Deserialize, Serialize};

/// Режим «авто» в `mode`/`next_mode`.
pub const MODE_AUTO: &str = "auto";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationView {
    /// `idle | needs_link | connecting | connected | no_host | interrupted`.
    pub state: String,
    /// Заголовок уведомления.
    pub text: String,
    /// Вторая строка: режим или причина.
    pub detail: Option<String>,
    /// Активный движок.
    pub engine: Option<String>,
    /// `auto` или `id` закреплённого движка.
    pub mode: String,
    /// Куда переключит действие «Движок».
    pub next_mode: String,
    /// Подпись действия: «Авто», «AWG», «CSQTT».
    pub next_label: String,
    /// Есть хотя бы один движок — действие «Движок» показывается.
    pub can_cycle: bool,
}

/// Подпись движка по виду или `id`: `awg` → «AWG», `csqtt` → «CSQTT».
pub fn engine_label(kind_or_id: &str) -> String {
    match kind_or_id {
        "awg" => "AWG".to_owned(),
        "csqtt" => "CSQTT".to_owned(),
        other => other.to_uppercase(),
    }
}

impl NotificationView {
    pub fn from_snapshot(s: &CoreSnapshot) -> Self {
        let (state, base, session_detail) = session_text(&s.session);
        let label_of = |id: &str| -> String {
            let kind = s
                .engines
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.kind.as_str())
                .unwrap_or(id);
            engine_label(kind)
        };

        // Движок в заголовке только там, где он что-то значит: «Подключено · AWG».
        let names_engine = matches!(s.session, SessionKind::Connected | SessionKind::Connecting);
        let text = match s.active.as_deref().filter(|_| names_engine) {
            Some(active) => format!("{} · {}", base.trim_end_matches('…'), label_of(active)),
            None => base,
        };

        let mode = s.pinned.clone().unwrap_or_else(|| MODE_AUTO.to_owned());
        // Цикл действия «Движок»: Авто → движки по приоритету → Авто.
        let order: Vec<&str> = std::iter::once(MODE_AUTO)
            .chain(s.engines.iter().map(|e| e.id.as_str()))
            .collect();
        let can_cycle = !s.engines.is_empty();
        let next_mode = if can_cycle {
            let at = order.iter().position(|m| *m == mode).unwrap_or(0);
            order[(at + 1) % order.len()].to_owned()
        } else {
            MODE_AUTO.to_owned()
        };
        let next_label = if next_mode == MODE_AUTO {
            "Авто".to_owned()
        } else {
            label_of(&next_mode)
        };

        let detail = session_detail.or_else(|| {
            can_cycle.then(|| {
                if mode == MODE_AUTO {
                    "Режим: Авто".to_owned()
                } else {
                    format!("Режим: закреплён {}", label_of(&mode))
                }
            })
        });

        Self {
            state: state.to_owned(),
            text,
            detail,
            engine: s.active.clone(),
            mode,
            next_mode,
            next_label,
            can_cycle,
        }
    }

    /// Режим для `CoreBridge::set_engine_mode` по `next_mode`.
    pub fn next_mode_sel(&self) -> EngineModeSel {
        if self.next_mode == MODE_AUTO {
            EngineModeSel::Auto
        } else {
            EngineModeSel::Pinned(self.next_mode.clone())
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("NotificationView сериализуется")
    }
}

/// Код состояния для Kotlin, заголовок и вторая строка, если её диктует состояние.
fn session_text(session: &SessionKind) -> (&'static str, String, Option<String>) {
    match session {
        SessionKind::Idle => ("idle", "Отключено".into(), None),
        SessionKind::NeedsLink => ("needs_link", "Нужна ссылка подписки".into(), None),
        SessionKind::Connecting => ("connecting", "Подключение…".into(), None),
        SessionKind::Connected => ("connected", "Подключено".into(), None),
        SessionKind::NoHost { last_error } => (
            "no_host",
            "Нет связи с хостом".into(),
            Some(last_error.clone()).filter(|e| !e.is_empty()),
        ),
        SessionKind::Interrupted { by_other_vpn: true } => (
            "interrupted",
            "VPN перехвачен другим приложением".into(),
            None,
        ),
        SessionKind::Interrupted {
            by_other_vpn: false,
        } => ("interrupted", "Разрешение VPN отозвано".into(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::android::bridge::EngineBrief;

    fn engines() -> Vec<EngineBrief> {
        vec![
            EngineBrief {
                id: "awg".into(),
                kind: "awg".into(),
            },
            EngineBrief {
                id: "csqtt".into(),
                kind: "csqtt".into(),
            },
        ]
    }

    fn snapshot(session: SessionKind, active: Option<&str>, pinned: Option<&str>) -> CoreSnapshot {
        CoreSnapshot {
            session,
            active: active.map(str::to_owned),
            pinned: pinned.map(str::to_owned),
            engines: engines(),
        }
    }

    #[test]
    fn connected_text_names_the_active_engine() {
        let v =
            NotificationView::from_snapshot(&snapshot(SessionKind::Connected, Some("awg"), None));
        assert_eq!(v.state, "connected");
        assert_eq!(v.text, "Подключено · AWG");
        assert_eq!(v.detail.as_deref(), Some("Режим: Авто"));
        assert_eq!(v.engine.as_deref(), Some("awg"));
        assert_eq!(v.mode, "auto");
    }

    #[test]
    fn engine_action_cycles_auto_awg_csqtt_auto() {
        let auto = NotificationView::from_snapshot(&snapshot(SessionKind::Connected, None, None));
        assert_eq!(auto.next_mode, "awg");
        assert_eq!(auto.next_label, "AWG");
        assert!(auto.can_cycle);

        let awg = NotificationView::from_snapshot(&snapshot(
            SessionKind::Connected,
            Some("awg"),
            Some("awg"),
        ));
        assert_eq!(awg.mode, "awg");
        assert_eq!(awg.detail.as_deref(), Some("Режим: закреплён AWG"));
        assert_eq!(awg.next_mode, "csqtt");
        assert_eq!(awg.next_label, "CSQTT");

        let csqtt = NotificationView::from_snapshot(&snapshot(
            SessionKind::Connected,
            Some("csqtt"),
            Some("csqtt"),
        ));
        assert_eq!(csqtt.next_mode, "auto");
        assert_eq!(csqtt.next_label, "Авто");
        assert_eq!(csqtt.next_mode_sel(), EngineModeSel::Auto);
        assert_eq!(awg.next_mode_sel(), EngineModeSel::Pinned("csqtt".into()));
    }

    #[test]
    fn without_engines_the_action_is_hidden() {
        let v = NotificationView::from_snapshot(&CoreSnapshot {
            session: SessionKind::NeedsLink,
            active: None,
            pinned: None,
            engines: vec![],
        });
        assert_eq!(v.state, "needs_link");
        assert_eq!(v.text, "Нужна ссылка подписки");
        assert!(!v.can_cycle);
        assert_eq!(v.next_mode, "auto");
    }

    #[test]
    fn interrupted_and_no_host_texts() {
        let other = NotificationView::from_snapshot(&snapshot(
            SessionKind::Interrupted { by_other_vpn: true },
            None,
            None,
        ));
        assert_eq!(other.state, "interrupted");
        assert_eq!(other.text, "VPN перехвачен другим приложением");

        let revoked = NotificationView::from_snapshot(&snapshot(
            SessionKind::Interrupted {
                by_other_vpn: false,
            },
            None,
            None,
        ));
        assert_eq!(revoked.text, "Разрешение VPN отозвано");

        let no_host = NotificationView::from_snapshot(&snapshot(
            SessionKind::NoHost {
                last_error: "таймаут пробы".into(),
            },
            None,
            None,
        ));
        assert_eq!(no_host.state, "no_host");
        assert_eq!(no_host.text, "Нет связи с хостом");
        assert_eq!(no_host.detail.as_deref(), Some("таймаут пробы"));
    }

    #[test]
    fn json_has_the_fields_kotlin_reads() {
        let v =
            NotificationView::from_snapshot(&snapshot(SessionKind::Connecting, Some("awg"), None));
        let j: serde_json::Value = serde_json::from_str(&v.to_json()).unwrap();
        assert_eq!(j["state"], "connecting");
        assert_eq!(j["text"], "Подключение · AWG");
        assert_eq!(j["next_label"], "AWG");
        assert_eq!(j["can_cycle"], true);
    }
}
