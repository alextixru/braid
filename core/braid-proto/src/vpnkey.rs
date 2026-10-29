//! Импорт ключей Amnezia `vpn://…` — заглушка, реализация в тикете 06.
//!
//! Формат (`notes-awg.md` §4.1): снять `vpn://`; base64url без `=` → байты;
//! первые 4 байта — длина распакованных данных (big-endian), далее zlib-поток
//! (`qCompress`); если распаковка пуста — байты уже JSON; `format_version` ≤ 1;
//! `containers[].awg.last_config` — вложенная JSON-строка с ключами клиента.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VpnKeyError {
    #[error("импорт ключей vpn:// ещё не реализован")]
    NotImplemented,
    #[error("это не ключ Amnezia (vpn://)")]
    NotVpnKey,
    #[error("ключ повреждён: {0}")]
    Malformed(String),
    #[error("неподдерживаемая версия ключа {0}")]
    UnsupportedVersion(u64),
}

/// Разобранный ключ Amnezia. Поля появятся вместе с реализацией.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmneziaKey {
    /// Сырой JSON ключа без секретов SSH админа.
    pub raw: serde_json::Value,
}

/// `vpn://…` → [`AmneziaKey`].
pub fn decode(_link: &str) -> Result<AmneziaKey, VpnKeyError> {
    Err(VpnKeyError::NotImplemented)
}
