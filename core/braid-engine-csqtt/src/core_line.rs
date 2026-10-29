//! Разбор строк лог-колбэка ядра: события `__CSQTT_EVENT__|<ТИП>|<json>`
//! и запрос капчи `CAPTCHA_SOLVE|<режим>|<url>|<сессия>`. Всё остальное —
//! обычные строки журнала, они не нужны.

const EVENT_PREFIX: &str = "__CSQTT_EVENT__|";
const CAPTCHA_PREFIX: &str = "CAPTCHA_SOLVE|";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreLine {
    /// `__CSQTT_EVENT__|<ТИП>|<json>` — STATS, ERROR, CONFIG, NETWORK_SUSPECT, CALL_UNAVAILABLE, …
    Event {
        kind: String,
        payload: serde_json::Value,
    },
    /// `CAPTCHA_SOLVE|<режим>|<url>|<сессия>` — ядро ждёт решение капчи.
    Captcha {
        mode: String,
        url: String,
        session: String,
    },
}

pub fn parse_core_line(line: &str) -> Option<CoreLine> {
    let line = line.trim_end();
    if let Some(rest) = line.strip_prefix(EVENT_PREFIX) {
        let (kind, json) = rest.split_once('|')?;
        let payload = serde_json::from_str(json)
            .or_else(|_| serde_json::from_str(strip_repeat_badge(json)))
            .ok()?;
        return Some(CoreLine::Event {
            kind: kind.to_owned(),
            payload,
        });
    }
    let line = strip_repeat_badge(line);
    if let Some(rest) = line.strip_prefix(CAPTCHA_PREFIX) {
        let mut fields = rest.splitn(3, '|');
        let mode = fields.next()?;
        let url = fields.next()?;
        let session = fields.next()?;
        return Some(CoreLine::Captcha {
            mode: mode.to_owned(),
            url: url.to_owned(),
            session: session.to_owned(),
        });
    }
    None
}

/// Логгер ядра схлопывает повторы в « (xN)» на конце строки — JSON после
/// такого бейджа не разбирается, срезаем его.
fn strip_repeat_badge(line: &str) -> &str {
    let Some(start) = line.rfind(" (x") else {
        return line;
    };
    let badge = &line[start + 3..];
    match badge.strip_suffix(')') {
        Some(count) if !count.is_empty() && count.bytes().all(|b| b.is_ascii_digit()) => {
            &line[..start]
        }
        _ => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_line_is_parsed_with_payload() {
        let line = "__CSQTT_EVENT__|ERROR|{\"code\":\"handshake_timeout\",\"fatal\":true,\"message\":\"FATAL_HANDSHAKE: no response\"}\n";
        match parse_core_line(line) {
            Some(CoreLine::Event { kind, payload }) => {
                assert_eq!(kind, "ERROR");
                assert_eq!(payload["code"], "handshake_timeout");
                assert_eq!(payload["fatal"], true);
            }
            other => panic!("ожидалось событие, получено {other:?}"),
        }
    }

    #[test]
    fn captcha_request_keeps_url_and_session() {
        let line = "CAPTCHA_SOLVE|auto|https://id.vk.com/captcha?session=1|sess-token-123\n";
        assert_eq!(
            parse_core_line(line),
            Some(CoreLine::Captcha {
                mode: "auto".to_owned(),
                url: "https://id.vk.com/captcha?session=1".to_owned(),
                session: "sess-token-123".to_owned(),
            })
        );
    }

    #[test]
    fn repeated_line_badge_is_stripped_before_json() {
        let line = "__CSQTT_EVENT__|NETWORK_SUSPECT|{} (x3)\n";
        assert_eq!(
            parse_core_line(line),
            Some(CoreLine::Event {
                kind: "NETWORK_SUSPECT".to_owned(),
                payload: serde_json::json!({}),
            })
        );
    }

    #[test]
    fn plain_log_lines_are_ignored() {
        assert_eq!(parse_core_line("[КЛИЕНТ] Все воркеры завершены\n"), None);
        assert_eq!(parse_core_line("__CSQTT_EVENT__|BROKEN"), None);
        assert_eq!(parse_core_line("__CSQTT_EVENT__|STATS|not json"), None);
        assert_eq!(parse_core_line("CAPTCHA_SOLVE|auto"), None);
    }
}
