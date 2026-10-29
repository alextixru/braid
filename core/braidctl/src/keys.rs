//! Ключи и секреты: пары x25519 в формате Amnezia (base64, как у WireGuard),
//! PSK, токены подписки (32 случайных байта base64url), пароли панели.
//! Значения никогда не логируются — наружу только через явные вызовы.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use braid_proto::Base64Key;
use rand::Rng as _;
use x25519_dalek::{PublicKey, StaticSecret};

pub struct KeyPair {
    pub private: Base64Key,
    pub public: Base64Key,
}

/// Алфавит паролей панели CSQTT (как у апстрима: без похожих символов).
pub const PASSWORD_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghjkmnpqrstuvwxyz23456789";

fn random_bytes() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

/// Приведение скаляра X25519 (RFC 7748 §5), как делает `wg genkey`.
pub fn clamp(key: &mut [u8; 32]) {
    key[0] &= 248;
    key[31] &= 127;
    key[31] |= 64;
}

pub fn generate_keypair() -> KeyPair {
    let mut bytes = random_bytes();
    clamp(&mut bytes);
    let private = Base64Key::from_bytes(bytes);
    KeyPair {
        public: public_key_of(&private),
        private,
    }
}

pub fn public_key_of(private: &Base64Key) -> Base64Key {
    let secret = StaticSecret::from(*private.as_bytes());
    Base64Key::from_bytes(PublicKey::from(&secret).to_bytes())
}

pub fn generate_psk() -> Base64Key {
    Base64Key::from_bytes(random_bytes())
}

/// Токен подписки: 32 случайных байта, base64url без набивки (43 символа).
pub fn generate_token() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes())
}

pub fn random_password(len: usize) -> String {
    (0..len)
        .map(|_| PASSWORD_CHARS[rand::random_range(0..PASSWORD_CHARS.len())] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Вектор RFC 7748 §6.1 (Алиса): приватный → публичный.
    #[test]
    fn public_key_matches_rfc_7748_vector() {
        let private: [u8; 32] =
            hex::decode("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a")
                .unwrap()
                .try_into()
                .unwrap();
        let public = public_key_of(&Base64Key::from_bytes(private));
        assert_eq!(
            public.to_hex(),
            "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"
        );
    }

    #[test]
    fn generated_keys_are_clamped_distinct_and_base64() {
        let a = generate_keypair();
        let b = generate_keypair();
        assert_ne!(a.private, b.private);
        let bytes = a.private.as_bytes();
        assert_eq!(bytes[0] & 7, 0);
        assert_eq!(bytes[31] & 128, 0);
        assert_eq!(bytes[31] & 64, 64);
        assert_eq!(a.private.to_base64().len(), 44);
        assert_eq!(public_key_of(&a.private), a.public);
    }

    #[test]
    fn token_is_43_url_safe_chars_from_32_bytes() {
        let token = generate_token();
        assert_eq!(token.len(), 43);
        assert!(
            token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_eq!(URL_SAFE_NO_PAD.decode(&token).unwrap().len(), 32);
        assert_ne!(token, generate_token());
    }

    #[test]
    fn password_uses_only_its_alphabet() {
        let p = random_password(24);
        assert_eq!(p.len(), 24);
        assert!(p.bytes().all(|b| PASSWORD_CHARS.contains(&b)));
    }
}
