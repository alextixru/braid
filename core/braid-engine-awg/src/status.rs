//! Разбор текста `awg_get_config` (`IpcGet` amneziawg-go): нам нужны
//! `listen_port`, `last_handshake_time_sec/nsec`, `tx_bytes`, `rx_bytes`
//! первого пира. Текст содержит ключи — разобрали и забыли, не логируем.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AwgStatus {
    pub listen_port: Option<u16>,
    /// Время последнего хендшейка; `None` — хендшейка ещё не было.
    pub last_handshake: Option<SystemTime>,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
}

/// Ключи устройства идут до первого `public_key`, дальше — пиры; берём первого.
pub fn parse_status(text: &str) -> AwgStatus {
    let mut st = AwgStatus::default();
    let mut peers = 0;
    let mut sec: u64 = 0;
    let mut nsec: u64 = 0;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key == "public_key" {
            peers += 1;
        }
        if peers > 1 {
            break;
        }
        match key {
            "listen_port" => st.listen_port = value.parse().ok(),
            "last_handshake_time_sec" => sec = value.parse().unwrap_or(0),
            "last_handshake_time_nsec" => nsec = value.parse().unwrap_or(0),
            "tx_bytes" => st.tx_bytes = value.parse().unwrap_or(0),
            "rx_bytes" => st.rx_bytes = value.parse().unwrap_or(0),
            _ => {}
        }
    }
    if sec > 0 {
        st.last_handshake = Some(UNIX_EPOCH + Duration::new(sec, nsec.min(999_999_999) as u32));
    }
    st
}

#[cfg(test)]
mod tests {
    use super::*;

    // Формат IpcGet (device/uapi.go): ключи устройства, затем блок пира.
    const SAMPLE: &str = "private_key=0101010101010101010101010101010101010101010101010101010101010101\n\
        listen_port=51820\n\
        jc=4\njmin=40\njmax=70\ns1=15\ns2=18\nh1=1\nh2=2\nh3=3\nh4=4\n\
        random_trailers=0\ndisable_cookies=0\n\
        public_key=8ce5809e3eb97d46e107676bb859742f695cef114e43d8c656aba7169924b660\n\
        preshared_key=0000000000000000000000000000000000000000000000000000000000000000\n\
        protocol_version=1\n\
        endpoint=203.0.113.10:51820\n\
        last_handshake_time_sec=1700000000\n\
        last_handshake_time_nsec=5\n\
        tx_bytes=1234\n\
        rx_bytes=5678\n\
        persistent_keepalive_interval=25\n\
        allowed_ip=0.0.0.0/0\n";

    #[test]
    fn parses_port_handshake_and_counters() {
        let st = parse_status(SAMPLE);
        assert_eq!(st.listen_port, Some(51820));
        assert_eq!(
            st.last_handshake,
            Some(UNIX_EPOCH + Duration::new(1_700_000_000, 5))
        );
        assert_eq!(st.tx_bytes, 1234);
        assert_eq!(st.rx_bytes, 5678);
    }

    #[test]
    fn zero_handshake_means_none() {
        let text = SAMPLE
            .replace(
                "last_handshake_time_sec=1700000000",
                "last_handshake_time_sec=0",
            )
            .replace("last_handshake_time_nsec=5", "last_handshake_time_nsec=0");
        let st = parse_status(&text);
        assert_eq!(st.last_handshake, None);
        assert_eq!(st.rx_bytes, 5678);
    }

    #[test]
    fn empty_or_garbage_gives_defaults() {
        assert_eq!(parse_status(""), AwgStatus::default());
        assert_eq!(
            parse_status("tx_bytes=abc\nlisten_port=99999\n"),
            AwgStatus::default()
        );
    }
}
