//! Проверка ввода пользователя на экране «Движки → CSQTT»: ссылка на звонок
//! VK (`https://vk.com/call/join/<хеш>`) или голый хеш. Ядро само принимает
//! почти любую строку как хеш, поэтому отсев кривого ввода (ист. 15) — здесь.

/// Текст ошибки для экрана (ист. 15).
pub const INVALID_CALL_LINK: &str = "Это не ссылка на звонок VK";

const JOIN_PATH: &str = "/call/join/";
const MIN_HASH_LEN: usize = 16;
const MAX_HASH_LEN: usize = 128;

/// Хеш звонка из ссылки или голого хеша; `None` — это не ссылка на звонок VK.
pub fn parse_call_hash(input: &str) -> Option<String> {
    let value = input.trim().trim_matches(['<', '>', '"', '\'']).trim();
    if value.is_empty() {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    if let Some(index) = lower.find(JOIN_PATH) {
        let host = lower[..index]
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        if !is_vk_host(host) {
            return None;
        }
        let tail = &value[index + JOIN_PATH.len()..];
        let end = tail.find(['?', '#', '/']).unwrap_or(tail.len());
        return hash_if_valid(&tail[..end]);
    }
    if lower.contains("://") || lower.contains('/') {
        return None;
    }
    hash_if_valid(value)
}

fn is_vk_host(host: &str) -> bool {
    let host = host.split('@').next_back().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    matches!(host, "vk.com" | "vk.ru") || host.ends_with(".vk.com") || host.ends_with(".vk.ru")
}

fn hash_if_valid(hash: &str) -> Option<String> {
    let valid_len = (MIN_HASH_LEN..=MAX_HASH_LEN).contains(&hash.len());
    let valid_chars = hash
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    (valid_len && valid_chars).then(|| hash.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "kw6VzMMQPO8fEtP0Yoo6_8nbuKfTCjxNq1GJdpQTnUA";

    #[test]
    fn call_links_normalize_to_the_hash() {
        for link in [
            format!("https://vk.com/call/join/{HASH}"),
            format!("https://vk.ru/call/join/{HASH}/"),
            format!("http://m.vk.com/call/join/{HASH}?utm=1#x"),
            format!("vk.com/call/join/{HASH}"),
            format!("  <https://vk.com/call/join/{HASH}>  "),
        ] {
            assert_eq!(parse_call_hash(&link).as_deref(), Some(HASH), "{link}");
        }
    }

    #[test]
    fn bare_hashes_are_accepted_as_is() {
        assert_eq!(parse_call_hash(HASH).as_deref(), Some(HASH));
        assert_eq!(
            parse_call_hash("abcdefghijklmnop").as_deref(),
            Some("abcdefghijklmnop")
        );
    }

    #[test]
    fn garbage_is_not_a_call_link() {
        for input in [
            "",
            "   ",
            "https://example.com/call/join/abc",
            "https://vk.com/id123",
            "https://vk.com/call/join/",
            "not a link",
            "short",
            "хеш-по-русски-длинный-очень",
            "https://vk.com/call/join/with space",
        ] {
            assert_eq!(parse_call_hash(input), None, "{input:?}");
        }
    }

    #[test]
    fn error_message_matches_the_spec() {
        assert_eq!(INVALID_CALL_LINK, "Это не ссылка на звонок VK");
    }
}
