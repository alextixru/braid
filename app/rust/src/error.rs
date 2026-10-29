//! Ошибки моста. Ядро говорит `CoreError`; Dart получает `BridgeError`:
//! вид (по нему экран выбирает, что предложить) и готовый русский текст
//! (его можно показать как есть).

use braid_core::rules::RulesError;
use braid_core::{CoreError, PlatformError};

/// Вид ошибки — что предложить пользователю.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeErrorKind {
    /// Ядро ещё не создано (Android: сервис запускается).
    NotReady,
    /// Подписки нет — нужна ссылка.
    NeedsLink,
    /// Ввод не похож на ссылку Braid.
    InvalidLink,
    /// Ссылка другого хоста: замена только после подтверждения.
    ReplaceRequested,
    /// Другой VPN держит маршруты и DNS.
    Conflict,
    /// Права не выданы: пользователь отказал или отозвано разрешение.
    Denied,
    /// Платформа этого не умеет.
    Unsupported,
    /// Действие только для администратора подписки.
    AdminOnly,
    /// В подписке нет ни одного поддерживаемого движка.
    NoEngines,
    /// Сессия не активна.
    NotConnected,
    /// Ввод не принят: не домен, нет такого набора, битый JSON команды.
    InvalidInput,
    /// Всё остальное; текст — в `message`.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeError {
    pub kind: BridgeErrorKind,
    /// Текст для экрана, по-русски.
    pub message: String,
    /// Подробности вида: `ReplaceRequested` — текущий и новый хост;
    /// `Conflict` — имя интерфейса.
    pub details: Vec<String>,
}

impl BridgeError {
    pub fn new(kind: BridgeErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            details: Vec::new(),
        }
    }

    pub fn with_details(mut self, details: Vec<String>) -> Self {
        self.details = details;
        self
    }

    pub fn not_ready() -> Self {
        Self::new(
            BridgeErrorKind::NotReady,
            "Ядро ещё запускается — подожди секунду",
        )
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(BridgeErrorKind::InvalidInput, message)
    }
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BridgeError {}

impl From<&CoreError> for BridgeError {
    fn from(error: &CoreError) -> Self {
        let message = error.to_string();
        match error {
            CoreError::NeedsLink => BridgeError::new(BridgeErrorKind::NeedsLink, message),
            CoreError::Link(_) => BridgeError::new(BridgeErrorKind::InvalidLink, message),
            CoreError::ReplaceRequested {
                current_host,
                new_host,
            } => BridgeError::new(BridgeErrorKind::ReplaceRequested, message)
                .with_details(vec![current_host.clone(), new_host.clone()]),
            CoreError::Platform(PlatformError::Conflict { iface }) => {
                BridgeError::new(BridgeErrorKind::Conflict, message)
                    .with_details(vec![iface.clone()])
            }
            CoreError::Platform(PlatformError::Denied(_)) => {
                BridgeError::new(BridgeErrorKind::Denied, message)
            }
            CoreError::Platform(PlatformError::Unsupported(_)) => {
                BridgeError::new(BridgeErrorKind::Unsupported, message)
            }
            CoreError::AdminOnly => BridgeError::new(BridgeErrorKind::AdminOnly, message),
            CoreError::NoEngines => BridgeError::new(BridgeErrorKind::NoEngines, message),
            CoreError::NotConnected => BridgeError::new(BridgeErrorKind::NotConnected, message),
            CoreError::Rules(
                RulesError::Domain(_) | RulesError::UnknownSet(_) | RulesError::UnknownMode(_),
            ) => BridgeError::new(BridgeErrorKind::InvalidInput, message),
            _ => BridgeError::new(BridgeErrorKind::Failed, message),
        }
    }
}

impl From<CoreError> for BridgeError {
    fn from(error: CoreError) -> Self {
        BridgeError::from(&error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use braid_core::proto::LinkError;

    fn kind_and_message(error: CoreError) -> (BridgeErrorKind, String, Vec<String>) {
        let mapped = BridgeError::from(error);
        (mapped.kind, mapped.message, mapped.details)
    }

    #[test]
    fn link_errors_are_shown_with_the_core_text() {
        let (kind, message, _) = kind_and_message(CoreError::Link(LinkError::NotBraid));
        assert_eq!(kind, BridgeErrorKind::InvalidLink);
        // Текст из критерия тикета 10: ввод не теряется, ошибка именно такая.
        assert_eq!(message, "Это не ссылка Braid");
    }

    #[test]
    fn missing_subscription_asks_for_a_link() {
        let (kind, message, _) = kind_and_message(CoreError::NeedsLink);
        assert_eq!(kind, BridgeErrorKind::NeedsLink);
        assert_eq!(message, "Сначала вставь ссылку или отсканируй QR");
    }

    #[test]
    fn replace_request_carries_both_hosts() {
        let (kind, _, details) = kind_and_message(CoreError::ReplaceRequested {
            current_host: "frankfurt.example".into(),
            new_host: "helsinki.example".into(),
        });
        assert_eq!(kind, BridgeErrorKind::ReplaceRequested);
        assert_eq!(details, vec!["frankfurt.example", "helsinki.example"]);
    }

    #[test]
    fn other_vpn_conflict_names_the_interface() {
        let (kind, message, details) =
            kind_and_message(CoreError::Platform(PlatformError::Conflict {
                iface: "utun5".into(),
            }));
        assert_eq!(kind, BridgeErrorKind::Conflict);
        assert_eq!(message, "Отключи utun5");
        assert_eq!(details, vec!["utun5"]);
    }

    #[test]
    fn refused_privileges_are_denied_not_failed() {
        let (kind, _, _) = kind_and_message(CoreError::Platform(PlatformError::Denied(
            "пользователь отменил запрос прав администратора".into(),
        )));
        assert_eq!(kind, BridgeErrorKind::Denied);
    }

    #[test]
    fn bad_user_input_is_invalid_input() {
        let (kind, _, _) = kind_and_message(CoreError::Rules(RulesError::UnknownSet("x".into())));
        assert_eq!(kind, BridgeErrorKind::InvalidInput);
        let (kind, _, _) = kind_and_message(CoreError::Rules(RulesError::UnknownMode("x".into())));
        assert_eq!(kind, BridgeErrorKind::InvalidInput);
    }

    #[test]
    fn simple_core_errors_map_to_their_kinds() {
        assert_eq!(
            kind_and_message(CoreError::AdminOnly).0,
            BridgeErrorKind::AdminOnly
        );
        assert_eq!(
            kind_and_message(CoreError::NoEngines).0,
            BridgeErrorKind::NoEngines
        );
        assert_eq!(
            kind_and_message(CoreError::NotConnected).0,
            BridgeErrorKind::NotConnected
        );
        assert_eq!(
            kind_and_message(CoreError::Io("диск".into())).0,
            BridgeErrorKind::Failed
        );
    }
}
